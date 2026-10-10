//! The report tail shared by every findings command: baseline, exit code, SARIF, and the one
//! write to stdout. The kit's own `scan` uses it; a tool's own command (its own clap `Command`,
//! so its own `--help` text) calls the same functions:
//!
//! ```ignore
//! let o = OutputArgs::from_matches(m, d);
//! let pre = preflight(d, &o)?;              // bad baseline refused before touching the subject
//! let code = finish(d, pre, d.try_scan(&ctx), &o);
//! ```
use crate::baseline::{self, Saved};
use crate::doctor::{is_scored, Doctor, ScanFailure};
use crate::face::{render, Face};
use crate::theme::Theme;
use clap::{Arg, ArgMatches, Args, ValueEnum};
use doctor_core::{Envelope, Severity};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq, Debug)]
pub enum Color {
    Auto,
    Always,
    Never,
}

/// The output flags of a findings command, defined once. Flattened into the kit's `scan`.
#[derive(Args)]
pub(crate) struct OutputFlags {
    /// Output face
    #[arg(long, value_enum, default_value = "rich")]
    pub face: Face,
    /// mono | clinical | contrast (default: the doctor's own theme)
    #[arg(long)]
    pub theme: Option<String>,
    /// theme.toml overlay (default: <path>/theme.toml when present)
    #[arg(long)]
    pub theme_file: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "auto")]
    pub color: Color,
    /// Print the doctor/1 envelope as JSON
    #[arg(long, conflicts_with_all = ["score", "headless"])]
    pub json: bool,
    /// Print only the score number
    #[arg(long, conflicts_with = "headless")]
    pub score: bool,
    /// Plain text, no color: for CI and pipes
    #[arg(long)]
    pub headless: bool,
    /// Also write a SARIF 2.1.0 report to this path
    #[arg(long)]
    pub sarif: Option<PathBuf>,
    /// Compare with a saved doctor/1 report: findings get baseline_state, exit 3 only for NEW ones
    #[arg(long)]
    pub baseline: Option<PathBuf>,
}

/// Parsed output options. Argument ids are the field names (`json`, `score`, `headless`, `face`,
/// `theme`, `theme_file`, `color`, `sarif`, `baseline`, `fail_on`).
#[derive(Debug, Clone)]
pub struct OutputArgs {
    pub json: bool,
    pub score: bool,
    pub headless: bool,
    pub face: Face,
    pub theme: Option<String>,
    pub theme_file: Option<PathBuf>,
    pub color: Color,
    pub sarif: Option<PathBuf>,
    pub baseline: Option<PathBuf>,
    pub fail_on: Severity,
    /// Where an implicit `theme.toml` is looked for. `None` (tools' own commands) = nowhere.
    pub theme_root: Option<PathBuf>,
    /// Findings never fail this command: exit 0 whatever their severity, with or without a baseline
    /// (a report-only command such as an audit). `fail_on` is then unused. See [`OutputArgs::never_fail`].
    pub never_fail: bool,
}

impl OutputArgs {
    /// The kit's standard definitions of these flags, for a tool's own `Command`:
    /// `--fail-on` defaults to `d.default_fail_on()`.
    pub fn args(d: &dyn Doctor) -> Vec<Arg> {
        Self::args_with(d.default_fail_on())
    }

    /// [`OutputArgs::args`] before the doctor exists (a tool defines its command first, then
    /// `run_with` builds the doctor): give the `--fail-on` default directly.
    pub fn args_with(fail_on: Severity) -> Vec<Arg> {
        let cmd = OutputFlags::augment_args(clap::Command::new("x"));
        let mut v: Vec<Arg> = cmd.get_arguments().cloned().collect();
        v.push(
            Arg::new("fail_on")
                .long("fail-on")
                .help("Exit 1 when a finding is at or above this severity")
                .default_value(fail_on.as_str())
                .value_parser(crate::cli::parse_sev),
        );
        v
    }

    /// Read the options from `m`. An id the command does not define takes its default, so a tool
    /// may offer a subset of the flags.
    pub fn from_matches(m: &ArgMatches, d: &dyn Doctor) -> OutputArgs {
        let flag = |id: &str| {
            m.try_get_one::<bool>(id)
                .ok()
                .flatten()
                .copied()
                .unwrap_or(false)
        };
        let path = |id: &str| m.try_get_one::<PathBuf>(id).ok().flatten().cloned();
        OutputArgs {
            json: flag("json"),
            score: flag("score"),
            headless: flag("headless"),
            face: m
                .try_get_one::<Face>("face")
                .ok()
                .flatten()
                .copied()
                .unwrap_or(Face::Rich),
            theme: m.try_get_one::<String>("theme").ok().flatten().cloned(),
            theme_file: path("theme_file"),
            color: m
                .try_get_one::<Color>("color")
                .ok()
                .flatten()
                .copied()
                .unwrap_or(Color::Auto),
            sarif: path("sarif"),
            baseline: path("baseline"),
            fail_on: m
                .try_get_one::<Severity>("fail_on")
                .ok()
                .flatten()
                .copied()
                .unwrap_or_else(|| d.default_fail_on()),
            theme_root: None,
            never_fail: false,
        }
    }

    /// Make findings never fail this command (exit 0): a per-command default for report-only commands.
    pub fn never_fail(mut self) -> Self {
        self.never_fail = true;
        self
    }
}

/// Work done before the subject is touched.
#[derive(Debug)]
pub struct Preflight {
    saved: Option<Saved>,
}

/// Load `--baseline` and refuse `--sarif` and `--baseline` naming the same file, before any scan.
pub fn preflight(d: &dyn Doctor, o: &OutputArgs) -> Result<Preflight, ScanFailure> {
    if let (Some(s), Some(b)) = (&o.sarif, &o.baseline) {
        let same = s == b
            || matches!((std::fs::canonicalize(s), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y);
        if same {
            return Err(d.preflight_error(
                crate::doctor::PreflightKind::SarifIsBaseline,
                format!(
                    "--sarif and --baseline name the same file ({}); the report would overwrite its own baseline",
                    b.display()
                ),
            ));
        }
    }
    let saved = match &o.baseline {
        Some(b) => Some(d.load_baseline(b)?),
        None => None,
    };
    Ok(Preflight { saved })
}

/// Print a failed scan: the envelope on stdout when there is one, then stderr verbatim.
pub fn report_failure(d: &dyn Doctor, f: &ScanFailure) -> u8 {
    if let Some(env) = &f.envelope {
        print!("{}", d.render_json(env));
    }
    eprint!("{}", f.stderr);
    f.exit
}

fn write_atomic(p: &Path, text: &str) -> std::io::Result<()> {
    let mut tmp = p.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, p))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

fn theme_of(d: &dyn Doctor, o: &OutputArgs) -> Result<Theme, String> {
    let mut t = match &o.theme {
        Some(n) => Theme::by_name(n)
            .ok_or_else(|| format!("unknown theme `{n}` (mono|clinical|contrast)"))?,
        None => d.theme(),
    };
    let file = o.theme_file.clone().or_else(|| {
        o.theme_root
            .as_ref()
            .map(|r| r.join("theme.toml"))
            .filter(|p| p.is_file())
    });
    if let Some(p) = file {
        t = t.with_toml(
            &std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?,
        )?;
    }
    t.enabled = !o.headless
        && match o.color {
            Color::Always => true,
            Color::Never => false,
            Color::Auto => {
                std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
            }
        };
    Ok(t)
}

/// Turn a scan result into output and an exit code.
///
/// On `Err`: print the failure ([`report_failure`]). On `Ok`: apply the baseline (and
/// [`Doctor::baseline_hook`]), set `env.exit_code` to the process exit code ([`baseline::gate`]:
/// 0, 1, or 3 under a baseline; always 0 with `never_fail`), write SARIF if asked (atomic; a write failure still prints the
/// report and exits 2), then write the report to stdout once.
pub fn finish(
    d: &dyn Doctor,
    pre: Preflight,
    r: Result<Envelope, ScanFailure>,
    o: &OutputArgs,
) -> u8 {
    let mut env = match r.and_then(|e| crate::interrupt::check(d).map(|_| e)) {
        Ok(e) => e,
        Err(f) => return report_failure(d, &f),
    };
    if let Some(saved) = &pre.saved {
        baseline::apply(d, &mut env, saved);
        if let Err(f) = d.baseline_hook(&saved.doc, &mut env) {
            return report_failure(d, &f);
        }
    }
    let mut code = if o.never_fail {
        0
    } else {
        baseline::gate(d, &env, o.fail_on, pre.saved.is_some())
    };
    env.exit_code = code;
    if let Some(p) = &o.sarif {
        if let Err(e) = write_atomic(p, &d.render_sarif(&env)) {
            eprintln!("error: {}: {e}", p.display());
            code = 2;
        }
    }
    let text = if o.json {
        d.render_json(&env)
    } else if o.score {
        format!(
            "{}\n",
            if is_scored(&env.score) {
                env.score.value.to_string()
            } else {
                "n/a".into()
            }
        )
    } else {
        let face = if o.headless { Face::Plain } else { o.face };
        match theme_of(d, o) {
            Ok(t) => {
                // The tool's own report gets the same sanitized copy the kit faces get.
                let safe = crate::face::sanitize_envelope(&env);
                d.render_human(&safe, &face, &t)
                    .unwrap_or_else(|| render(face, &env, &t))
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    };
    print!("{text}");
    code
}
