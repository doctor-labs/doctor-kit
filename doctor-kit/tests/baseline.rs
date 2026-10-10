//! K2: baseline load/apply/gate.
use doctor_core::{BaselineState, Envelope, Finding, Location, LocationKind, Score, Severity};
use doctor_kit::baseline::{self, BaselineError};
use doctor_kit::{Check, Doctor};
use serde_json::json;
use std::path::PathBuf;

struct D;
impl Doctor for D {
    fn name(&self) -> &str {
        "t-doctor"
    }
    fn version(&self) -> &str {
        "1"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec![]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![]
    }
    fn is_gap(&self, f: &Finding) -> bool {
        f.id.starts_with("gap.")
    }
}

fn fnd(id: &str, fp: &str, sev: Severity) -> Finding {
    Finding {
        id: id.into(),
        fingerprint: fp.into(),
        severity: sev,
        confidence: None,
        category: "c".into(),
        message: "m".into(),
        location: Location::new(LocationKind::File, "x"),
        evidence: vec![],
        remedy: None,
        baseline_state: None,
        extra: Default::default(),
    }
}
fn env(fs: Vec<Finding>) -> Envelope {
    Envelope::new("t-doctor", "1", 0, Score::new(100, "t/1", 0), fs, json!({}))
}
fn file(tag: &str, text: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("bl-{tag}-{}.json", std::process::id()));
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn new_unchanged_fixed_and_exit_codes() {
    let base = env(vec![
        fnd("a", "fp-a", Severity::High),
        fnd("old", "fp-old", Severity::High),
    ]);
    let p = file("env", &serde_json::to_string(&base).unwrap());
    let saved = baseline::load(&p, 1 << 20).unwrap();

    // Same findings: nothing new, one fixed, exit 0 even though high >= fail_on.
    let mut e = env(vec![fnd("a", "fp-a", Severity::High)]);
    let c = baseline::apply(&D, &mut e, &saved);
    assert_eq!((c.new, c.unchanged, c.fixed), (0, 1, 1));
    assert_eq!(e.findings[0].baseline_state, Some(BaselineState::Unchanged));
    assert_eq!(e.baseline, Some(c));
    assert_eq!(baseline::gate(&D, &e, Severity::High, true), 0);
    assert_eq!(
        baseline::gate(&D, &e, Severity::High, false),
        1,
        "no baseline: any >= fail_on"
    );

    // A new finding below fail_on does not gate; at or above it exits 3.
    let mut e = env(vec![
        fnd("a", "fp-a", Severity::High),
        fnd("n", "fp-n", Severity::Low),
    ]);
    baseline::apply(&D, &mut e, &saved);
    assert_eq!(baseline::gate(&D, &e, Severity::High, true), 0);
    let mut e = env(vec![fnd("n", "fp-n", Severity::Critical)]);
    let c = baseline::apply(&D, &mut e, &saved);
    assert_eq!((c.new, c.unchanged, c.fixed), (1, 0, 2));
    assert_eq!(baseline::gate(&D, &e, Severity::High, true), 3);
}

#[test]
fn gaps_are_never_unchanged_and_never_gate_under_a_baseline() {
    let p = file("gap", r#"[{"fingerprint":"fp-g"}]"#);
    let saved = baseline::load(&p, 1 << 20).unwrap();
    let mut e = env(vec![fnd("gap.x", "fp-g", Severity::Critical)]);
    let c = baseline::apply(&D, &mut e, &saved);
    assert_eq!((c.new, c.unchanged), (1, 0));
    assert_eq!(e.findings[0].baseline_state, Some(BaselineState::New));
    assert_eq!(baseline::gate(&D, &e, Severity::High, true), 0);
    assert_eq!(
        baseline::gate(&D, &e, Severity::High, false),
        1,
        "without a baseline a gap gates like any finding"
    );
}

#[test]
fn fingerprints_match_exactly_as_written() {
    let p = file(
        "fp",
        r#"{"schema":"doctor/1","findings":[{"fingerprint":"Weird FP/1"},{"fingerprint":""}]}"#,
    );
    let saved = baseline::load(&p, 1 << 20).unwrap();
    let mut e = env(vec![
        fnd("a", "Weird FP/1", Severity::High),
        fnd("b", "weird fp/1", Severity::High),
    ]);
    let c = baseline::apply(&D, &mut e, &saved);
    assert_eq!((c.new, c.unchanged, c.fixed), (1, 1, 1));
}

#[test]
fn rejects_what_is_not_a_doctor1_report() {
    for (tag, text) in [
        ("obj", r#"{"findings":[]}"#),
        ("schema", r#"{"schema":"other/1","findings":[]}"#),
        ("nofp", r#"[{"id":"x"}]"#),
        ("scalar", "3"),
    ] {
        let e = baseline::load(&file(tag, text), 1 << 20).unwrap_err();
        assert!(matches!(e, BaselineError::NotDoctor1 { .. }), "{tag}: {e}");
    }
    assert!(matches!(
        baseline::load(&file("junk", "{nope"), 1 << 20).unwrap_err(),
        BaselineError::NotJson { .. }
    ));
    assert!(matches!(
        baseline::load(&file("big", "[]"), 1).unwrap_err(),
        BaselineError::TooLarge { max: 1, .. }
    ));
    assert!(matches!(
        baseline::load(std::path::Path::new("/nonexistent/bl.json"), 10).unwrap_err(),
        BaselineError::Unreadable { .. }
    ));
}

#[test]
fn message_and_cap_are_the_doctors() {
    struct Own;
    impl Doctor for Own {
        fn name(&self) -> &str {
            "own"
        }
        fn version(&self) -> &str {
            "1"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec![]
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![]
        }
        fn baseline_max_bytes(&self) -> usize {
            1
        }
        fn baseline_message(&self, e: &BaselineError) -> String {
            format!("own says: {e:?}").chars().take(18).collect()
        }
    }
    let f = baseline::load_for(&Own, &file("own", "[]")).unwrap_err();
    assert_eq!(f.exit, 2);
    assert_eq!(f.stderr, "error: own says: TooLarge\n");
    // default wording names the file
    let f = baseline::load_for(&D, &file("def", "{nope")).unwrap_err();
    assert!(f.stderr.contains("not valid JSON"), "{}", f.stderr);
}

mod hooks {
    use super::*;
    use doctor_kit::{OutputArgs, PreflightKind, ScanFailure};

    /// ble-like: a bare array is NOT a baseline, entries without a fingerprint are skipped, no size cap,
    /// no "error: " prefix on stderr.
    struct Strict;
    impl Doctor for Strict {
        fn name(&self) -> &str {
            "strict"
        }
        fn version(&self) -> &str {
            "1"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec![]
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![]
        }
        fn load_baseline(&self, path: &std::path::Path) -> Result<baseline::Saved, ScanFailure> {
            let bytes =
                std::fs::read(path).map_err(|e| ScanFailure::plain(format!("baseline: {e}")))?;
            let doc: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| ScanFailure::plain(format!("baseline: {e}")))?;
            let list = doc
                .get("findings")
                .and_then(|f| f.as_array())
                .ok_or_else(|| ScanFailure::plain("baseline: not a doctor/1 report"))?;
            let fingerprints = list
                .iter()
                .filter_map(|f| f.get("fingerprint").and_then(|v| v.as_str()))
                .map(String::from)
                .collect();
            Ok(baseline::Saved { fingerprints, doc })
        }
        fn preflight_error(&self, kind: PreflightKind, msg: String) -> ScanFailure {
            match kind {
                PreflightKind::SarifIsBaseline => ScanFailure::plain(format!("same-file: {msg}")),
                _ => ScanFailure::new(msg),
            }
        }
    }

    fn opts(baseline: Option<PathBuf>, sarif: Option<PathBuf>) -> OutputArgs {
        let m = clap::Command::new("x")
            .args(OutputArgs::args(&Strict))
            .try_get_matches_from(
                ["x".to_string()]
                    .into_iter()
                    .chain(
                        baseline
                            .iter()
                            .flat_map(|b| ["--baseline".into(), b.display().to_string()]),
                    )
                    .chain(
                        sarif
                            .iter()
                            .flat_map(|b| ["--sarif".into(), b.display().to_string()]),
                    ),
            )
            .unwrap();
        OutputArgs::from_matches(&m, &Strict)
    }

    #[test]
    fn a_tools_loader_replaces_the_kit_loader_in_preflight() {
        // Accepted by the kit loader (bare array), refused by the tool's.
        let bare = file("hook-bare", r#"[{"fingerprint":"a"}]"#);
        assert!(doctor_kit::preflight(&D, &opts(Some(bare.clone()), None)).is_ok());
        let e = doctor_kit::preflight(&Strict, &opts(Some(bare), None)).unwrap_err();
        assert_eq!(
            (e.exit, e.stderr.as_str()),
            (2, "baseline: not a doctor/1 report\n")
        );
        // Skips entries with no fingerprint instead of rejecting the file.
        let ok = file(
            "hook-ok",
            r#"{"schema":"doctor/1","findings":[{"id":"x"},{"fingerprint":"b"}]}"#,
        );
        assert!(doctor_kit::preflight(&Strict, &opts(Some(ok.clone()), None)).is_ok());
        let e = doctor_kit::preflight(&D, &opts(Some(ok), None)).unwrap_err();
        assert!(
            e.stderr.starts_with("error: ") && e.stderr.contains("not a doctor/1 report"),
            "{}",
            e.stderr
        );
    }

    #[test]
    fn preflight_failures_go_through_preflight_error() {
        let same = file("hook-same", "{}");
        let e = doctor_kit::preflight(&Strict, &opts(Some(same.clone()), Some(same.clone())))
            .unwrap_err();
        assert!(
            e.stderr.starts_with("same-file: --sarif and --baseline"),
            "no kit prefix: {}",
            e.stderr
        );
        let e = doctor_kit::preflight(&D, &opts(Some(same.clone()), Some(same))).unwrap_err();
        assert!(
            e.stderr.starts_with("error: --sarif and --baseline"),
            "{}",
            e.stderr
        );
        assert_eq!(ScanFailure::plain("x").stderr, "x\n");
    }
}
