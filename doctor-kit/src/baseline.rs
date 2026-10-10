//! `--baseline`: compare a scan with a saved doctor/1 report (contract sections 4 and 6).
//! Findings get `baseline_state` (`new` / `unchanged`), the envelope gets `baseline` counts, and
//! with a baseline the exit code is 3 only when a NEW finding reaches `--fail-on`.
//! Matching is by `fingerprint` exactly as the tool wrote it; the kit never recomputes one.
use crate::doctor::{Doctor, ScanFailure};
use doctor_core::{BaselineState, Envelope, Severity};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::path::Path;

pub use doctor_core::BaselineCounts as Counts;

/// A parsed baseline file.
#[derive(Debug, Clone)]
pub struct Saved {
    pub fingerprints: HashSet<String>,
    /// The whole document, for [`Doctor::baseline_hook`] (rule drift, dialect checks, diffs).
    pub doc: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineError {
    Unreadable { path: String, err: String },
    TooLarge { path: String, max: usize },
    NotJson { path: String, err: String },
    NotDoctor1 { path: String },
    Other(String),
}

impl fmt::Display for BaselineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BaselineError::Unreadable { path, err } => write!(f, "--baseline {path}: {err}"),
            BaselineError::TooLarge { path, max } => {
                write!(f, "--baseline {path}: larger than {max} bytes")
            }
            BaselineError::NotJson { path, err } => {
                write!(f, "--baseline {path}: not valid JSON: {err}")
            }
            BaselineError::NotDoctor1 { path } => write!(
                f,
                "--baseline {path}: not a doctor/1 report (expected the output of `scan --json`)"
            ),
            BaselineError::Other(m) => f.write_str(m),
        }
    }
}

/// Read a baseline: a doctor/1 envelope, or a bare array of findings (older tools wrote that).
pub fn load(path: &Path, max_bytes: usize) -> Result<Saved, BaselineError> {
    let p = path.display().to_string();
    let meta = std::fs::metadata(path).map_err(|e| BaselineError::Unreadable {
        path: p.clone(),
        err: e.to_string(),
    })?;
    if meta.len() > max_bytes as u64 {
        return Err(BaselineError::TooLarge {
            path: p,
            max: max_bytes,
        });
    }
    let bytes = std::fs::read(path).map_err(|e| BaselineError::Unreadable {
        path: p.clone(),
        err: e.to_string(),
    })?;
    let doc: Value = serde_json::from_slice(&bytes).map_err(|e| BaselineError::NotJson {
        path: p.clone(),
        err: e.to_string(),
    })?;
    let list = match &doc {
        Value::Array(a) => Some(a),
        Value::Object(o) if o.get("schema").and_then(Value::as_str) == Some("doctor/1") => {
            o.get("findings").and_then(Value::as_array)
        }
        _ => None,
    };
    let bad = || BaselineError::NotDoctor1 { path: p.clone() };
    let mut fingerprints = HashSet::new();
    for f in list.ok_or_else(bad)? {
        let fp = f
            .get("fingerprint")
            .and_then(Value::as_str)
            .ok_or_else(bad)?;
        fingerprints.insert(fp.to_string());
    }
    Ok(Saved { fingerprints, doc })
}

/// Load for `d`: size cap and message wording come from the doctor.
pub fn load_for(d: &dyn Doctor, path: &Path) -> Result<Saved, ScanFailure> {
    load(path, d.baseline_max_bytes()).map_err(|e| ScanFailure::new(d.baseline_message(&e)))
}

/// Set `baseline_state` on every finding and `env.baseline`. A coverage gap ([`Doctor::is_gap`])
/// is never `unchanged`: not evaluating a rule is not the same as it having been accepted.
pub fn apply(d: &dyn Doctor, env: &mut Envelope, saved: &Saved) -> Counts {
    let mut seen = HashSet::new();
    let (mut new, mut unchanged) = (0, 0);
    for f in &mut env.findings {
        seen.insert(f.fingerprint.clone());
        if saved.fingerprints.contains(&f.fingerprint) && !d.is_gap(f) {
            f.baseline_state = Some(BaselineState::Unchanged);
            unchanged += 1;
        } else {
            f.baseline_state = Some(BaselineState::New);
            new += 1;
        }
    }
    let fixed = saved.fingerprints.difference(&seen).count();
    let c = Counts {
        new,
        unchanged,
        fixed,
    };
    env.baseline = Some(c);
    c
}

/// The exit code: without a baseline, 1 when any finding is at or above `fail_on`; with one, 3 when
/// a NEW non-gap finding is. Otherwise 0.
pub fn gate(d: &dyn Doctor, env: &Envelope, fail_on: Severity, baselined: bool) -> u8 {
    if baselined {
        let hit = env.findings.iter().any(|f| {
            f.severity >= fail_on
                && f.baseline_state != Some(BaselineState::Unchanged)
                && !d.is_gap(f)
        });
        if hit {
            3
        } else {
            0
        }
    } else {
        env.findings.iter().any(|f| f.severity >= fail_on) as u8
    }
}
