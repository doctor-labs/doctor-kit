//! `run::<D>()`: the whole CLI. Always: `scan`, `fix`, `install`, `ci install`.
//! Feature `repl`: `shell`; `tui`: `explore`; `mcp`: `mcp`.
use crate::check::{Ctx, Scope};
use crate::config::Config;
use crate::doctor::{Doctor, TargetKind};
use crate::install::Agent;
use clap::{ArgMatches, Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use doctor_core::Severity;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scan a target and print a report
    Scan(ScanArgs),
    /// Interactive shell over the same session
    #[cfg(feature = "repl")]
    Shell(ShellArgs),
    /// Full-screen explorer over the same session
    #[cfg(feature = "tui")]
    Explore(Target),
    /// MCP stdio server exposing `scan` and `fix` as tools
    #[cfg(feature = "mcp")]
    Mcp,
    /// Print (or copy) a prompt for a coding agent from one finding or the whole scan
    Fix(FixArgs),
    /// Write an agent skill, Cursor rule and AGENTS.md block into a repo
    Install(InstallArgs),
    /// CI helpers
    Ci(CiArgs),
}

#[derive(Args)]
struct FixArgs {
    /// Finding rule id or fingerprint prefix (default: every finding)
    id: Option<String>,
    /// Directory to scan
    #[arg(short = 'C', long, default_value = ".")]
    path: PathBuf,
    #[command(flatten)]
    opts: Opts,
    /// Also copy the prompt to the clipboard
    #[arg(long)]
    copy: bool,
    /// Take the findings from this saved `scan --json` report instead of scanning
    #[arg(long)]
    from: Option<PathBuf>,
    /// Hand the prompt to this coding agent instead of printing it
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(crate::fix::AGENTS.map(|a| a.name)))]
    agent: Option<String>,
    /// With --agent: skip the agent's approval prompts (the scanned text is untrusted)
    #[arg(long, requires = "agent")]
    yolo: bool,
}

#[derive(Args)]
struct InstallArgs {
    /// Repository to write into
    #[arg(long, default_value = ".")]
    dir: PathBuf,
    /// One agent only (default: all)
    #[arg(long, value_enum)]
    agent: Option<Agent>,
    /// Also add a Claude Code PostToolUse hook that scans changed files after edits
    #[arg(long)]
    agent_hooks: bool,
    /// Print the files instead of writing them
    #[arg(long)]
    print_only: bool,
}

#[derive(Args)]
struct CiArgs {
    #[command(subcommand)]
    action: CiAction,
}

#[derive(Subcommand)]
enum CiAction {
    /// Write a GitHub Actions workflow that scans pull requests
    Install {
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// Print the workflow instead of writing it
        #[arg(long)]
        print: bool,
        /// Replace an existing workflow whose content differs
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args)]
struct Target {
    /// Directory to examine
    #[arg(default_value = ".")]
    path: PathBuf,
    #[command(flatten)]
    opts: Opts,
}

/// What a scan looks at; shared by `scan`, `fix`, `shell`, `explore` and the MCP tools.
#[derive(Args)]
pub(crate) struct Opts {
    /// full, or only changed files (worktree vs HEAD, untracked, and `--base`)
    #[arg(long, value_enum, default_value = "full")]
    pub scope: Scope,
    /// PR mode: with `--scope changed`, also files changed since merge-base(BASE, HEAD)
    #[arg(long)]
    pub base: Option<String>,
    /// doctor.toml (default: <path>/doctor.toml when present)
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Exit 1 when a finding is at or above this severity
    #[arg(long, default_value = "high", value_parser = parse_sev)]
    pub fail_on: Severity,
}

pub(crate) fn parse_sev(s: &str) -> Result<Severity, String> {
    Severity::parse(s)
        .ok_or_else(|| format!("expected one of info|low|medium|high|critical, got `{s}`"))
}

#[derive(Args)]
struct ScanArgs {
    #[command(flatten)]
    target: Target,
    #[command(flatten)]
    out: crate::output::OutputFlags,
}

#[cfg(feature = "repl")]
#[derive(Args)]
struct ShellArgs {
    #[command(flatten)]
    target: Target,
    /// Run these `;`-separated commands and exit
    #[arg(short = 'c')]
    command: Option<String>,
    /// Print every reply as one JSON record (for agents)
    #[arg(long)]
    json: bool,
}

fn ctx_of(d: &dyn Doctor, t: &Target) -> Result<Ctx, String> {
    ctx_from(d, &t.path, &t.opts)
}

/// The scan context for `d`: a walked directory, or for a `Detached` doctor nothing at all (only an
/// explicit `--config` is read; no `doctor.toml` is looked for in the working directory).
pub(crate) fn ctx_from(d: &dyn Doctor, path: &Path, o: &Opts) -> Result<Ctx, String> {
    let detached = d.target() == TargetKind::Detached;
    let cfg_path = o
        .config
        .clone()
        .or_else(|| Some(path.join("doctor.toml")).filter(|p| !detached && p.is_file()));
    let config = match cfg_path {
        Some(p) => Config::from_toml(
            &std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?,
        )?,
        None => Config::default(),
    };
    if detached {
        return Ok(Ctx::detached(config, o.fail_on));
    }
    Ctx::with_walk(
        path,
        o.scope,
        o.base.clone(),
        config,
        o.fail_on,
        &d.walk_options(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))
}

fn fix(d: &dyn Doctor, a: FixArgs) -> Result<u8, String> {
    let env = match &a.from {
        Some(p) => crate::fix::load_envelope(p)?,
        None => d
            .try_scan(&ctx_from(d, &a.path, &a.opts)?)
            .map_err(|f| f.to_string())?,
    };
    let hits = crate::fix::select(&env, a.id.as_deref())?;
    if hits.is_empty() {
        warnln!("nothing to fix");
        return Ok(0);
    }
    let prompt = d.fix_prompt(&env, &hits);
    if a.copy {
        sayln!("{prompt}")?;
        crate::fix::copy(&prompt)?;
        warnln!("copied to clipboard");
    }
    let spec = a
        .agent
        .as_deref()
        .and_then(|n| crate::fix::AGENTS.iter().find(|s| s.name == n));
    match spec {
        None => {
            if !a.copy {
                sayln!("{prompt}")?;
            }
            Ok(0)
        }
        Some(spec) => hand_off(d, spec, &prompt, a.yolo),
    }
}

/// Start the agent with the prompt; print the prompt instead when we are inside an agent or the
/// agent is not installed.
fn hand_off(
    d: &dyn Doctor,
    spec: &crate::fix::AgentSpec,
    prompt: &str,
    yolo: bool,
) -> Result<u8, String> {
    let me = d.command();
    if crate::fix::in_agent(|k| std::env::var(k).ok(), &[]) {
        warnln!(
            "{me}: running inside an agent; not launching {}. Prompt follows.",
            spec.bin
        );
        sayln!("{prompt}")?;
        return Ok(0);
    }
    if crate::fix::find_on_path(spec.bin, std::env::var_os("PATH")).is_none() {
        warnln!(
            "{me}: {} is not on PATH; printing the prompt instead",
            spec.bin
        );
        sayln!("{prompt}")?;
        return Ok(0);
    }
    if yolo {
        warnln!(
            "{me}: WARNING launching {} with approvals skipped ({}); the scanned text is untrusted",
            spec.bin,
            spec.bypass
        );
    }
    crate::fix::launch(spec, prompt, yolo).map(|_| 0)
}

fn install(d: &dyn Doctor, a: InstallArgs) -> Result<u8, String> {
    if a.print_only {
        for f in crate::install::plan(d, a.agent) {
            sayln!(
                "=== {} ===\n{}",
                f.path.display(),
                crate::install::rendered(&f)
            )?;
        }
        if a.agent_hooks {
            sayln!(
                "=== .claude/settings.json ===\n{}",
                crate::install::hook_settings("", &d.command())?
            )?;
        }
        return Ok(0);
    }
    for p in crate::install::install(d, &a.dir, a.agent, a.agent_hooks)? {
        sayln!("wrote {}", p.display())?;
    }
    Ok(0)
}

fn ci(d: &dyn Doctor, a: CiArgs) -> Result<u8, String> {
    let CiAction::Install { dir, print, force } = a.action;
    if print {
        say!("{}", d.ci_workflow())?;
    } else {
        let ow = if force {
            crate::install::Overwrite::Always
        } else {
            d.ci_overwrite()
        };
        sayln!(
            "wrote {}",
            crate::install::ci_install_with(d, &dir, ow)?.display()
        )?;
    }
    Ok(0)
}

fn scan(d: &dyn Doctor, a: ScanArgs) -> Result<u8, String> {
    crate::interrupt::install();
    let o = crate::output::OutputArgs {
        json: a.out.json,
        score: a.out.score,
        headless: a.out.headless,
        face: a.out.face,
        theme: a.out.theme,
        theme_file: a.out.theme_file,
        color: a.out.color,
        sarif: a.out.sarif,
        baseline: a.out.baseline,
        fail_on: a.target.opts.fail_on,
        theme_root: (d.target() == TargetKind::Dir).then(|| a.target.path.clone()),
    };
    let pre = match crate::output::preflight(d, &o) {
        Ok(p) => p,
        Err(f) => return Ok(crate::output::report_failure(d, &f)),
    };
    let ctx = ctx_of(d, &a.target)?;
    Ok(crate::output::finish(d, pre, d.try_scan(&ctx), &o))
}

/// What the command tree needs to know about a doctor before it exists (the doctor may be built
/// from the parsed arguments, so help and parsing cannot ask it).
#[derive(Debug, Clone)]
pub struct Meta {
    pub name: String,
    pub version: String,
    pub about: String,
    /// Default of `--fail-on` on the kit's commands.
    pub fail_on: Severity,
    pub target: TargetKind,
}

impl Meta {
    pub fn of(d: &dyn Doctor) -> Meta {
        Meta {
            name: d.name().into(),
            version: d.version().into(),
            about: d.about().into(),
            fail_on: d.default_fail_on(),
            target: d.target(),
        }
    }
}

/// A tool's own subcommand. One named like a kit command (`scan`, `fix`, `install`, `ci`, `mcp`,
/// `shell`, `explore`) replaces it, in the same place in `--help`. `run` gets that subcommand's matches.
pub struct ExtCommand<D> {
    pub command: clap::Command,
    pub run: fn(&D, &ArgMatches) -> Result<u8, String>,
}

/// How a tool extends the kit's CLI.
pub struct Extensions<D> {
    pub commands: Vec<ExtCommand<D>>,
    /// Arguments accepted before or after any subcommand (e.g. `--adapter`); made `global`.
    pub global_args: Vec<clap::Arg>,
    /// Exit code for a command line clap rejected (the error is printed first). Default: clap's own.
    pub usage_exit: fn(&clap::Error, &[String]) -> u8,
}

impl<D> Default for Extensions<D> {
    fn default() -> Self {
        Extensions {
            commands: vec![],
            global_args: vec![],
            usage_exit: |e, _| e.exit_code() as u8,
        }
    }
}

/// The command tree: the kit's commands, with the tool's replacing or joining them.
pub fn command_with<D>(meta: &Meta, ext: &Extensions<D>) -> clap::Command {
    let mut cmd = Cli::command()
        .name(meta.name.clone())
        .bin_name(meta.name.clone())
        .version(meta.version.clone())
        .about(meta.about.clone());
    // `--fail-on` defaults to the doctor's own severity (help shows it too); a detached doctor has
    // no directory, so the walk options are hidden (and ignored).
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();
    for n in names {
        cmd = cmd.mut_subcommand(n, |mut c| {
            if c.get_arguments().any(|a| a.get_id() == "fail_on") {
                c = c.mut_arg("fail_on", |a| a.default_value(meta.fail_on.as_str()));
            }
            if meta.target == TargetKind::Detached {
                for id in ["path", "scope", "base"] {
                    if c.get_arguments().any(|a| a.get_id() == id) {
                        c = c.mut_arg(id, |a| a.hide(true));
                    }
                }
            }
            c
        });
    }
    for e in &ext.commands {
        let name = e.command.get_name().to_string();
        if cmd.find_subcommand(&name).is_some() {
            cmd = cmd.mut_subcommand(name, |_| e.command.clone());
        } else {
            cmd = cmd.subcommand(e.command.clone());
        }
    }
    for a in &ext.global_args {
        cmd = cmd.arg(a.clone().global(true));
    }
    cmd
}

#[cfg(test)]
fn command_for<D: Doctor>(d: &D) -> clap::Command {
    command_with(&Meta::of(d), &Extensions::<D>::default())
}

fn builtin(d: &dyn Doctor, cli: Cli) -> Result<u8, String> {
    match cli.cmd {
        Cmd::Scan(a) => scan(d, a),
        Cmd::Fix(a) => fix(d, a),
        Cmd::Install(a) => install(d, a),
        Cmd::Ci(a) => ci(d, a),
        #[cfg(feature = "mcp")]
        Cmd::Mcp => crate::mcp::serve(d).map(|_| 0).map_err(|e| e.to_string()),
        #[cfg(feature = "repl")]
        Cmd::Shell(a) => {
            ctx_of(d, &a.target).and_then(|c| crate::repl::run(d, c, a.command.as_deref(), a.json))
        }
        #[cfg(feature = "tui")]
        Cmd::Explore(t) => ctx_of(d, &t).and_then(|c| crate::tui::run(d, c)).map(|_| 0),
    }
}

/// Run the CLI with the tool's extensions. `meta` describes the doctor for `--help` and parsing;
/// `build` creates the doctor from the root matches once the command line parsed (look inside
/// `m.subcommand()` for the subcommand's own arguments).
/// argv as UTF-8. A non-UTF-8 argument is a usage error (exit 2 at the call site), never a panic.
fn utf8_argv(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Vec<String>, String> {
    args.into_iter()
        .enumerate()
        .map(|(i, a)| {
            a.into_string().map_err(|a| {
                format!(
                    "argument {} is not valid UTF-8: {}",
                    i + 1,
                    a.to_string_lossy()
                )
            })
        })
        .collect()
}

pub fn run_with<D: Doctor>(
    meta: Meta,
    ext: Extensions<D>,
    build: impl FnOnce(&ArgMatches) -> Result<D, String>,
) -> ExitCode {
    let argv = match utf8_argv(std::env::args_os()) {
        Ok(a) => a,
        Err(e) => {
            warnln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let matches = match command_with(&meta, &ext).try_get_matches_from(&argv) {
        Ok(m) => m,
        Err(e) => {
            let code = (ext.usage_exit)(&e, &argv);
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    let doctor = match build(&matches) {
        Ok(d) => d,
        Err(e) => {
            warnln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let sub = matches.subcommand();
    let res = match sub.and_then(|(n, m)| {
        ext.commands
            .iter()
            .find(|c| c.command.get_name() == n)
            .map(|c| (c, m))
    }) {
        Some((c, m)) => (c.run)(&doctor, m),
        None => match Cli::from_arg_matches(&matches) {
            Ok(cli) => builtin(&doctor, cli),
            Err(e) => {
                let _ = e.print();
                return ExitCode::from(2);
            }
        },
    };
    match res {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            warnln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

/// The whole CLI for a doctor: `scan`, `fix`, `install`, `ci`, and with features `mcp`, `shell`,
/// `explore`. Name, version and about come from the doctor, not from doctor-kit's Cargo metadata.
pub fn run<D: Doctor>(doctor: D) -> ExitCode {
    let meta = Meta::of(&doctor);
    run_with(meta, Extensions::default(), move |_| Ok(doctor))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Named;
    impl Doctor for Named {
        fn name(&self) -> &str {
            "foo-doctor"
        }
        fn version(&self) -> &str {
            "9.9.9"
        }
        fn about(&self) -> &str {
            "Scans foo"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec![]
        }
        fn checks(&self) -> Vec<Box<dyn crate::check::Check>> {
            vec![]
        }
    }

    #[test]
    fn help_and_version_come_from_the_doctor() {
        let mut cmd = command_for(&Named);
        let help = cmd.render_help().to_string();
        assert!(
            help.contains("Scans foo") && help.contains("foo-doctor"),
            "{help}"
        );
        assert!(!help.contains("Reusable runtime"), "{help}");
        assert_eq!(cmd.render_version().trim(), "foo-doctor 9.9.9");
    }

    struct Strict;
    impl Doctor for Strict {
        fn name(&self) -> &str {
            "strict-doctor"
        }
        fn version(&self) -> &str {
            "1"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec![]
        }
        fn checks(&self) -> Vec<Box<dyn crate::check::Check>> {
            vec![]
        }
        fn default_fail_on(&self) -> Severity {
            Severity::Critical
        }
    }

    #[test]
    fn fail_on_default_is_the_doctors_on_every_command() {
        let cmd = command_for(&Strict);
        for sub in ["scan", "fix"] {
            let help = cmd
                .clone()
                .find_subcommand_mut(sub)
                .unwrap()
                .render_help()
                .to_string();
            assert!(help.contains("[default: critical]"), "{sub}: {help}");
        }
        let help = command_for(&Named)
            .find_subcommand_mut("scan")
            .unwrap()
            .render_help()
            .to_string();
        assert!(help.contains("[default: high]"), "{help}");
        let m = command_for(&Strict)
            .try_get_matches_from(["x", "scan", "."])
            .unwrap();
        let (_, sm) = m.subcommand().unwrap();
        assert_eq!(sm.get_one::<Severity>("fail_on"), Some(&Severity::Critical));
    }
}

#[cfg(all(test, unix))]
mod argv_tests {
    use super::utf8_argv;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn non_utf8_argument_is_a_usage_error_not_a_panic() {
        let bad = OsString::from_vec(vec![b's', b'c', 0xff]);
        let err = utf8_argv(vec![OsString::from("tool"), bad]).unwrap_err();
        assert!(err.starts_with("argument 2 is not valid UTF-8"), "{err}");
    }

    #[test]
    fn utf8_argv_passes_through() {
        let v = utf8_argv(vec![OsString::from("tool"), OsString::from("scan")]).unwrap();
        assert_eq!(v, vec!["tool", "scan"]);
    }
}
