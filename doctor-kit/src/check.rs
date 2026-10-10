//! `Ctx` (what a scan sees) and the `Check` trait (modeled on expo-doctor + brew: explicit
//! registration, per-check applicability, findings plus optional advice).
use crate::config::Config;
use doctor_core::{fingerprint, sanitize, Finding, Location, LocationKind, Severity};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Scope {
    Full,
    /// Files in `git diff HEAD`. Falls back to full when git is unavailable.
    Changed,
}

/// Everything a check may look at. File-based doctors use `files`; others ignore it and
/// open their own target from `root`.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub root: PathBuf,
    /// Paths relative to `root`, sorted.
    pub files: Vec<PathBuf>,
    pub config: Config,
    pub scope: Scope,
    pub fail_on: Severity,
}

impl Ctx {
    pub fn new(
        root: impl Into<PathBuf>,
        scope: Scope,
        config: Config,
        fail_on: Severity,
    ) -> std::io::Result<Ctx> {
        let root: PathBuf = root.into();
        let mut files = Vec::new();
        walk(&root, Path::new(""), &mut files)?;
        if scope == Scope::Changed {
            // ponytail: git-diff stub; staged/untracked/--base are not handled yet.
            if let Some(changed) = git_changed(&root) {
                files.retain(|f| changed.contains(f));
            }
        }
        files.sort();
        Ok(Ctx {
            root,
            files,
            config,
            scope,
            fail_on,
        })
    }

    /// Read a file under `root` as UTF-8 (None for binary or unreadable files).
    pub fn read(&self, rel: &Path) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel)).ok()
    }
}

fn walk(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(root.join(rel))? {
        let e = e?;
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        let p = rel.join(&name);
        // file_type does not follow symlinks, so links are skipped.
        match e.file_type()? {
            t if t.is_dir() => walk(root, &p, out)?,
            t if t.is_file() => out.push(p),
            _ => {}
        }
    }
    Ok(())
}

fn git_changed(root: &Path) -> Option<Vec<PathBuf>> {
    let o = std::process::Command::new("git")
        .args(["diff", "--name-only", "--relative", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    o.status.success().then(|| {
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(PathBuf::from)
            .collect()
    })
}

#[derive(Debug, Default)]
pub struct CheckResult {
    pub findings: Vec<Finding>,
    /// Shown by `why`; one line of guidance for the whole check.
    pub advice: Option<String>,
}

impl CheckResult {
    pub fn ok() -> Self {
        Self::default()
    }
    pub fn findings(findings: Vec<Finding>) -> Self {
        CheckResult {
            findings,
            advice: None,
        }
    }
    pub fn advice(mut self, a: impl Into<String>) -> Self {
        self.advice = Some(a.into());
        self
    }
}

pub trait Check {
    /// Stable rule id; `doctor.toml` enables/disables by this.
    fn id(&self) -> &'static str;
    fn category(&self) -> &'static str;
    /// Gate: skipped (and listed under `data.skipped`) when false.
    fn applies(&self, _ctx: &Ctx) -> bool {
        true
    }
    fn run(&self, ctx: &Ctx) -> CheckResult;
}

/// Build a finding with a stable fingerprint (`id|file|line`) and a sanitized message.
/// Set `.remedy` / `.evidence` on the result as needed.
pub fn finding(
    id: &str,
    category: &str,
    severity: Severity,
    message: &str,
    file: &Path,
    line: Option<u32>,
) -> Finding {
    let reference = file.to_string_lossy().replace('\\', "/");
    let fp = fingerprint(format!("{id}|{reference}|{}", line.unwrap_or(0)).as_bytes());
    let mut location = Location::new(LocationKind::File, reference);
    location.line = line;
    Finding {
        id: id.into(),
        fingerprint: fp,
        severity,
        confidence: None,
        category: category.into(),
        message: sanitize(message),
        location,
        evidence: vec![],
        remedy: None,
        baseline_state: None,
        extra: Default::default(),
    }
}
