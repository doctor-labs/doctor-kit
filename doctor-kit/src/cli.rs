//! `run::<D>()`: the whole CLI. Always: `scan`, `fix`, `install`, `ci install`.
//! Feature `repl`: `shell`; `tui`: `explore`; `mcp`: `mcp`.
use crate::check::{Ctx, Scope};
use crate::config::Config;
use crate::doctor::{is_scored, Doctor};
use crate::face::{render, Face};
use crate::install::Agent;
use crate::theme::Theme;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use doctor_core::{Envelope, Severity};
use std::io::IsTerminal;
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

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Color {
    Auto,
    Always,
    Never,
}

#[derive(Args)]
struct ScanArgs {
    #[command(flatten)]
    target: Target,
    /// Output face
    #[arg(long, value_enum, default_value = "rich")]
    face: Face,
    /// mono | clinical | contrast (default: the doctor's own theme)
    #[arg(long)]
    theme: Option<String>,
    /// theme.toml overlay (default: <path>/theme.toml when present)
    #[arg(long)]
    theme_file: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "auto")]
    color: Color,
    /// Print the doctor/1 envelope as JSON
    #[arg(long, conflicts_with_all = ["score", "headless"])]
    json: bool,
    /// Print only the score number
    #[arg(long, conflicts_with = "headless")]
    score: bool,
    /// Plain text, no color: for CI and pipes
    #[arg(long)]
    headless: bool,
    /// Also write a SARIF 2.1.0 report to this path
    #[arg(long)]
    sarif: Option<PathBuf>,
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

fn ctx_of(t: &Target) -> Result<Ctx, String> {
    ctx_from(&t.path, &t.opts)
}

pub(crate) fn ctx_from(path: &Path, o: &Opts) -> Result<Ctx, String> {
    let cfg_path = o
        .config
        .clone()
        .or_else(|| Some(path.join("doctor.toml")).filter(|p| p.is_file()));
    let config = match cfg_path {
        Some(p) => Config::from_toml(
            &std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?,
        )?,
        None => Config::default(),
    };
    Ctx::with_base(path, o.scope, o.base.clone(), config, o.fail_on)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The exact text `scan --json` prints (also the MCP `scan` result).
pub(crate) fn envelope_text(env: &Envelope) -> String {
    serde_json::to_string_pretty(env).unwrap() + "\n"
}

fn fix(d: &dyn Doctor, a: FixArgs) -> Result<u8, String> {
    let env = d.scan(&ctx_from(&a.path, &a.opts)?);
    let hits = crate::fix::select(&env, a.id.as_deref())?;
    if hits.is_empty() {
        eprintln!("nothing to fix");
        return Ok(0);
    }
    let prompt = d.fix_prompt(&env, &hits);
    println!("{prompt}");
    if a.copy {
        crate::fix::copy(&prompt)?;
        eprintln!("copied to clipboard");
    }
    Ok(0)
}

fn install(d: &dyn Doctor, a: InstallArgs) -> Result<u8, String> {
    for p in crate::install::install(d, &a.dir, a.agent, a.agent_hooks)? {
        println!("wrote {}", p.display());
    }
    Ok(0)
}

fn ci(d: &dyn Doctor, a: CiArgs) -> Result<u8, String> {
    let CiAction::Install { dir, print } = a.action;
    if print {
        print!("{}", d.ci_workflow());
    } else {
        println!("wrote {}", crate::install::ci_install(d, &dir)?.display());
    }
    Ok(0)
}

fn theme_of(base: Theme, a: &ScanArgs) -> Result<Theme, String> {
    let mut t = match &a.theme {
        Some(n) => Theme::by_name(n)
            .ok_or_else(|| format!("unknown theme `{n}` (mono|clinical|contrast)"))?,
        None => base,
    };
    let file = a
        .theme_file
        .clone()
        .or_else(|| Some(a.target.path.join("theme.toml")).filter(|p| p.is_file()));
    if let Some(p) = file {
        t = t.with_toml(
            &std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?,
        )?;
    }
    t.enabled = !a.headless
        && match a.color {
            Color::Always => true,
            Color::Never => false,
            Color::Auto => {
                std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
            }
        };
    Ok(t)
}

fn scan(d: &dyn Doctor, a: ScanArgs) -> Result<u8, String> {
    let ctx = ctx_of(&a.target)?;
    let env = d.scan(&ctx);
    if let Some(p) = &a.sarif {
        let v = crate::session::to_sarif(&env);
        std::fs::write(p, serde_json::to_string_pretty(&v).unwrap() + "\n")
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    if a.json {
        print!("{}", envelope_text(&env));
    } else if a.score {
        println!(
            "{}",
            if is_scored(&env.score) {
                env.score.value.to_string()
            } else {
                "n/a".into()
            }
        );
    } else {
        let face = if a.headless { Face::Plain } else { a.face };
        print!("{}", render(face, &env, &theme_of(d.theme(), &a)?));
    }
    Ok(env.exit_code)
}

fn command_for<D: Doctor>(d: &D) -> clap::Command {
    Cli::command()
        .name(d.name().to_string())
        .bin_name(d.name().to_string())
        .version(d.version().to_string())
        .about(d.about().to_string())
}

pub fn run<D: Doctor>(doctor: D) -> ExitCode {
    // Name, version and about come from the doctor, not from doctor-kit's own Cargo metadata.
    let matches = command_for(&doctor).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let res = match cli.cmd {
        Cmd::Scan(a) => scan(&doctor, a),
        Cmd::Fix(a) => fix(&doctor, a),
        Cmd::Install(a) => install(&doctor, a),
        Cmd::Ci(a) => ci(&doctor, a),
        #[cfg(feature = "mcp")]
        Cmd::Mcp => crate::mcp::serve(&doctor)
            .map(|_| 0)
            .map_err(|e| e.to_string()),
        #[cfg(feature = "repl")]
        Cmd::Shell(a) => ctx_of(&a.target)
            .and_then(|c| crate::repl::run(&doctor, c, a.command.as_deref(), a.json)),
        #[cfg(feature = "tui")]
        Cmd::Explore(t) => ctx_of(&t)
            .and_then(|c| crate::tui::run(&doctor, c))
            .map(|_| 0),
    };
    match res {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
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
}
