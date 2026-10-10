//! `run::<D>()`: the whole CLI. `scan` always; `shell` (feature `repl`), `explore` (feature `tui`).
use crate::check::{Ctx, Scope};
use crate::config::Config;
use crate::doctor::{is_scored, Doctor};
use crate::face::{render, Face};
use crate::theme::Theme;
use clap::{Args, Parser, Subcommand, ValueEnum};
use doctor_core::Severity;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(version, about)]
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
}

#[derive(Args)]
struct Target {
    /// Directory to examine
    #[arg(default_value = ".")]
    path: PathBuf,
    /// full, or only files in `git diff HEAD`
    #[arg(long, value_enum, default_value = "full")]
    scope: Scope,
    /// doctor.toml (default: <path>/doctor.toml when present)
    #[arg(long)]
    config: Option<PathBuf>,
    /// Exit 1 when a finding is at or above this severity
    #[arg(long, default_value = "high", value_parser = parse_sev)]
    fail_on: Severity,
}

fn parse_sev(s: &str) -> Result<Severity, String> {
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
}

#[cfg(feature = "repl")]
#[derive(Args)]
struct ShellArgs {
    #[command(flatten)]
    target: Target,
    /// Run these `;`-separated commands and exit
    #[arg(short = 'c')]
    command: Option<String>,
}

fn ctx_of(t: &Target) -> Result<Ctx, String> {
    let cfg_path = t
        .config
        .clone()
        .or_else(|| Some(t.path.join("doctor.toml")).filter(|p| p.is_file()));
    let config = match cfg_path {
        Some(p) => Config::from_toml(
            &std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?,
        )?,
        None => Config::default(),
    };
    Ctx::new(&t.path, t.scope, config, t.fail_on).map_err(|e| format!("{}: {e}", t.path.display()))
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
    if a.json {
        println!("{}", serde_json::to_string_pretty(&env).unwrap());
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

pub fn run<D: Doctor>(doctor: D) -> ExitCode {
    let res = match Cli::parse().cmd {
        Cmd::Scan(a) => scan(&doctor, a),
        #[cfg(feature = "repl")]
        Cmd::Shell(a) => ctx_of(&a.target)
            .and_then(|c| crate::repl::run(&doctor, c, a.command.as_deref()))
            .map(|_| 0),
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
