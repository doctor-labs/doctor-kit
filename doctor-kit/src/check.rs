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
    /// PR mode: with `Scope::Changed`, also include files changed since `merge-base(base, HEAD)`.
    pub base: Option<String>,
    pub fail_on: Severity,
}

impl Ctx {
    pub fn new(
        root: impl Into<PathBuf>,
        scope: Scope,
        config: Config,
        fail_on: Severity,
    ) -> std::io::Result<Ctx> {
        Self::with_base(root, scope, None, config, fail_on)
    }

    pub fn with_base(
        root: impl Into<PathBuf>,
        scope: Scope,
        base: Option<String>,
        config: Config,
        fail_on: Severity,
    ) -> std::io::Result<Ctx> {
        Self::with_walk(root, scope, base, config, fail_on, &WalkOptions::default())
    }

    /// `with_base` with the directory walk chosen by the tool (see [`WalkOptions`]).
    pub fn with_walk(
        root: impl Into<PathBuf>,
        scope: Scope,
        base: Option<String>,
        config: Config,
        fail_on: Severity,
        walk_opts: &WalkOptions,
    ) -> std::io::Result<Ctx> {
        let root: PathBuf = root.into();
        let mut files = Vec::new();
        walk(&root, Path::new(""), &mut files, walk_opts)?;
        if scope == Scope::Changed {
            if let Some(changed) = git_changed(&root, base.as_deref()) {
                files.retain(|f| changed.contains(f));
            }
        }
        files.sort();
        Ok(Ctx {
            root,
            files,
            config,
            scope,
            base,
            fail_on,
        })
    }

    /// A context that looks at nothing on disk: root ".", no files, full scope. For doctors whose
    /// subject is not a directory (`TargetKind::Detached`). Cannot fail.
    pub fn detached(config: Config, fail_on: Severity) -> Ctx {
        Ctx {
            root: PathBuf::from("."),
            files: Vec::new(),
            config,
            scope: Scope::Full,
            base: None,
            fail_on,
        }
    }

    /// Read a file under `root` as UTF-8 (None for binary or unreadable files).
    pub fn read(&self, rel: &Path) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel)).ok()
    }
}

/// What the directory walk includes. The default is the 0.2.2 walk: dot-entries, `target` and
/// `node_modules` are skipped without a trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkOptions {
    include_hidden: bool,
    skip: Vec<String>,
    report_skipped: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        WalkOptions {
            include_hidden: false,
            skip: vec!["target".into(), "node_modules".into()],
            report_skipped: false,
        }
    }
}

impl WalkOptions {
    pub fn new() -> Self {
        Self::default()
    }
    /// Walk dot-named files and directories too (default: skipped).
    pub fn include_hidden(mut self, yes: bool) -> Self {
        self.include_hidden = yes;
        self
    }
    /// Exact entry names to skip, replacing the default `target` and `node_modules`. Dot-names
    /// stay governed by `include_hidden`.
    pub fn skip_names<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.skip = names.into_iter().map(Into::into).collect();
        self
    }
    /// Add an info note to `data.notes` naming the skipped entries (default: no note). The note
    /// is not a coverage gap: no INCOMPLETE banner and no score change.
    pub fn report_skipped(mut self, yes: bool) -> Self {
        self.report_skipped = yes;
        self
    }
    pub fn reports_skipped(&self) -> bool {
        self.report_skipped
    }
    fn skips(&self, name: &str) -> bool {
        (!self.include_hidden && name.starts_with('.')) || self.skip.iter().any(|s| s == name)
    }
}

fn walk(
    root: &Path,
    rel: &Path,
    out: &mut Vec<PathBuf>,
    opts: &WalkOptions,
) -> std::io::Result<()> {
    for e in std::fs::read_dir(root.join(rel))? {
        let e = e?;
        let name = e.file_name();
        if opts.skips(&name.to_string_lossy()) {
            continue;
        }
        let p = rel.join(&name);
        // file_type does not follow symlinks, so links are skipped.
        match e.file_type()? {
            t if t.is_dir() => walk(root, &p, out, opts)?,
            t if t.is_file() => out.push(p),
            _ => {}
        }
    }
    Ok(())
}

/// Entries that the walk under `opts` drops, relative to `root`, sorted. A skipped directory is
/// one entry; its contents are not listed.
pub fn skipped_entries(root: &Path, opts: &WalkOptions) -> std::io::Result<Vec<PathBuf>> {
    fn go(
        root: &Path,
        rel: &Path,
        opts: &WalkOptions,
        out: &mut Vec<PathBuf>,
    ) -> std::io::Result<()> {
        for e in std::fs::read_dir(root.join(rel))? {
            let e = e?;
            let name = e.file_name();
            let p = rel.join(&name);
            if opts.skips(&name.to_string_lossy()) {
                out.push(p);
            } else if e.file_type()?.is_dir() {
                go(root, &p, opts, out)?;
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(root, Path::new(""), opts, &mut out)?;
    out.sort();
    Ok(out)
}

/// Changed files: worktree vs HEAD, untracked, and (PR mode) `base...HEAD`. `None` when git
/// is unavailable or this is not a repo, so the scan falls back to full.
fn git_changed(root: &Path, base: Option<&str>) -> Option<Vec<PathBuf>> {
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()?;
        o.status.success().then(|| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        })
    };
    let mut out = git(&["diff", "--name-only", "--relative", "HEAD"])?;
    out.extend(git(&["ls-files", "--others", "--exclude-standard"]).unwrap_or_default());
    if let Some(b) = base {
        let range = format!("{b}...HEAD");
        out.extend(git(&["diff", "--name-only", "--relative", &range]).unwrap_or_default());
    }
    Some(out)
}

/// Per-rule SARIF metadata, from `Doctor::rule_meta`. Unset fields are left out of the rule.
#[derive(Debug, Clone, Default)]
pub struct RuleMeta {
    name: Option<String>,
    short: Option<String>,
    full: Option<String>,
    help_uri: Option<String>,
    level: Option<Severity>,
}

impl RuleMeta {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn name(mut self, v: impl Into<String>) -> Self {
        self.name = Some(v.into());
        self
    }
    pub fn short_description(mut self, v: impl Into<String>) -> Self {
        self.short = Some(v.into());
        self
    }
    pub fn full_description(mut self, v: impl Into<String>) -> Self {
        self.full = Some(v.into());
        self
    }
    pub fn help_uri(mut self, v: impl Into<String>) -> Self {
        self.help_uri = Some(v.into());
        self
    }
    /// `defaultConfiguration.level` (SARIF level of the severity).
    pub fn default_level(mut self, s: Severity) -> Self {
        self.level = Some(s);
        self
    }
    pub(crate) fn to_json(&self, id: &str) -> serde_json::Value {
        let mut r = serde_json::json!({"id": id});
        if let Some(v) = &self.name {
            r["name"] = v.as_str().into();
        }
        if let Some(v) = &self.short {
            r["shortDescription"] = serde_json::json!({"text": v});
        }
        if let Some(v) = &self.full {
            r["fullDescription"] = serde_json::json!({"text": v});
        }
        if let Some(v) = &self.help_uri {
            r["helpUri"] = v.as_str().into();
        }
        if let Some(v) = self.level {
            r["defaultConfiguration"] = serde_json::json!({"level": v.sarif_level()});
        }
        r
    }
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
    /// `run` plus keyed JSON for `envelope.data` (see [`CheckData`]). Default: `run` and no data.
    /// A check that contributes data overrides this and implements `run` as `self.run_data(ctx).0`.
    fn run_data(&self, ctx: &Ctx) -> (CheckResult, CheckData) {
        (self.run(ctx), CheckData::new())
    }
}

/// Keyed JSON one check contributes to `envelope.data`, merged after the kit's own keys in
/// registration order: a later check with the same key replaces the earlier value. A key the kit
/// reserves (`doctor::RESERVED_DATA_KEYS`) is ignored, and the collision is an info note in
/// `data.notes`.
pub type CheckData = serde_json::Map<String, serde_json::Value>;

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
