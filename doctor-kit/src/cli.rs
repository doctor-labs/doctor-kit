//! `run::<D>()`: the whole CLI. Always: `scan`, `fix`, `install`, `ci install`.
//! Feature `repl`: `shell`; `tui`: `explore`; `mcp`: `mcp`.
use crate::check::{Ctx, Scope};
use crate::config::Config;
use crate::doctor::Doctor;
use crate::install::Agent;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
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

fn fix(d: &dyn Doctor, a: FixArgs) -> Result<u8, String> {
    let env = d
        .try_scan(&ctx_from(&a.path, &a.opts)?)
        .map_err(|f| f.to_string())?;
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

fn scan(d: &dyn Doctor, a: ScanArgs) -> Result<u8, String> {
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
        theme_root: Some(a.target.path.clone()),
    };
    let pre = match crate::output::preflight(d, &o) {
        Ok(p) => p,
        Err(f) => return Ok(crate::output::report_failure(d, &f)),
    };
    let ctx = ctx_of(&a.target)?;
    Ok(crate::output::finish(d, pre, d.try_scan(&ctx), &o))
}

fn command_for<D: Doctor>(d: &D) -> clap::Command {
    let default_fail_on = d.default_fail_on();
    let mut cmd = Cli::command()
        .name(d.name().to_string())
        .bin_name(d.name().to_string())
        .version(d.version().to_string())
        .about(d.about().to_string());
    // `--fail-on` defaults to the doctor's own severity (help shows it too).
    let names: Vec<String> = cmd
        .get_subcommands()
        .filter(|c| c.get_arguments().any(|a| a.get_id() == "fail_on"))
        .map(|c| c.get_name().to_string())
        .collect();
    for n in names {
        cmd = cmd.mut_subcommand(n, |c| {
            c.mut_arg("fail_on", |a| a.default_value(default_fail_on.as_str()))
        });
    }
    cmd
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
