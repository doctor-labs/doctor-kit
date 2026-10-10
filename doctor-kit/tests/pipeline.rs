//! K1: the scan pipeline hooks (collect, try_scan, failure, render_json, detached subject).
use doctor_core::{Finding, Location, LocationKind, Score, Severity};
use doctor_kit::{
    failure_envelope, finding, Check, CheckResult, Collected, Config, Ctx, Doctor, ScanFailure,
    TargetKind,
};
use serde_json::{json, Value};
use std::path::Path;

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

/// A device-style doctor: own findings, own data, own fingerprints, own JSON, can fail.
struct Dev {
    fail: bool,
    fps: Vec<&'static str>,
}
impl Doctor for Dev {
    fn name(&self) -> &str {
        "dev-doctor"
    }
    fn version(&self) -> &str {
        "1.2.3"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec![]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![]
    }
    fn target(&self) -> TargetKind {
        TargetKind::Detached
    }
    fn default_fail_on(&self) -> Severity {
        Severity::Critical
    }
    fn is_gap(&self, f: &Finding) -> bool {
        f.id.starts_with("gap.")
    }
    fn score(&self, _: &[Finding], _: &Ctx) -> Option<Score> {
        Some(Score::new(77, "dev/1", 0))
    }
    fn score_collected(&self, c: &Collected, _: &Ctx) -> Option<Score> {
        Some(Score::new(77, "dev/1", c.gaps))
    }
    fn render_json(&self, env: &doctor_core::Envelope) -> String {
        serde_json::to_string(&env.to_value()).unwrap() + "\n"
    }
    fn collect(&self, _: &Ctx) -> Result<Collected, ScanFailure> {
        if self.fail {
            return Err(
                ScanFailure::new("no reader").with_envelope(failure_envelope(
                    self,
                    "no-reader",
                    "no reader",
                    json!({"scanned": false}),
                )),
            );
        }
        Ok(Collected {
            findings: self
                .fps
                .iter()
                .enumerate()
                .map(|(i, fp)| fnd(&format!("r.{i}"), fp, Severity::High))
                .collect(),
            data: json!({"mine": true}),
            gaps: 2,
        })
    }
}

#[test]
fn collect_override_owns_data_and_gaps() {
    let d = Dev {
        fail: false,
        fps: vec!["a"],
    };
    let ctx = Ctx::detached(Config::default(), Severity::Critical);
    let env = d.try_scan(&ctx).unwrap();
    assert_eq!(env.data, json!({"mine": true}));
    assert_eq!(env.score.coverage_gaps, 2);
    assert_eq!(env.score.model, "dev/1");
    assert_eq!(env.exit_code, 0, "High is below critical");
}

#[test]
fn fingerprints_pass_through_untouched() {
    let weird = [
        "abc",
        "",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ];
    let d = Dev {
        fail: false,
        fps: weird.to_vec(),
    };
    let env = d.scan(&Ctx::detached(Config::default(), Severity::High));
    let got: Vec<&str> = env
        .findings
        .iter()
        .map(|f| f.fingerprint.as_str())
        .collect();
    let mut want = weird.to_vec();
    want.sort();
    let mut got_sorted = got.clone();
    got_sorted.sort();
    assert_eq!(got_sorted, want);
    let text = d.render_json(&env);
    let v: Value = serde_json::from_str(&text).unwrap();
    for fp in weird {
        assert!(v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["fingerprint"] == fp));
    }
    assert!(!text.trim_end().contains('\n'), "tool-chosen compact JSON");
}

#[test]
fn failure_with_envelope_and_infallible_scan() {
    let d = Dev {
        fail: true,
        fps: vec![],
    };
    let ctx = Ctx::detached(Config::default(), Severity::High);
    let f = d.try_scan(&ctx).unwrap_err();
    assert_eq!((f.exit, f.stderr.as_str()), (2, "error: no reader\n"));
    assert_eq!(f.to_string(), "no reader");
    let env = *f.envelope.unwrap();
    assert_eq!(env.score.value, 0);
    assert_eq!(env.score.label, "critical");
    assert_eq!(env.score.coverage_gaps, 1);
    assert_eq!(env.score.model, "dev/1");
    assert!(env.findings.is_empty());
    assert_eq!(
        env.data["error"],
        json!({"kind": "no-reader", "message": "no reader"})
    );
    assert_eq!(env.data["scanned"], false);
    // `scan` never fails: same envelope.
    assert_eq!(d.scan(&ctx).data["error"]["kind"], "no-reader");
}

#[test]
fn failure_without_envelope_becomes_scan_failed() {
    struct Bad;
    impl Doctor for Bad {
        fn name(&self) -> &str {
            "bad"
        }
        fn version(&self) -> &str {
            "0"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec![]
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![]
        }
        fn collect(&self, _: &Ctx) -> Result<Collected, ScanFailure> {
            Err("boom".to_string().into())
        }
    }
    let env = Bad.scan(&Ctx::detached(Config::default(), Severity::High));
    assert_eq!(
        env.data["error"],
        json!({"kind": "scan-failed", "message": "boom"})
    );
    assert_eq!(env.score.model, "doctor-kit/1");
}

#[test]
fn detached_ctx_walks_nothing_and_defaults_are_unchanged() {
    let ctx = Ctx::detached(Config::default(), Severity::Low);
    assert!(ctx.files.is_empty());
    assert_eq!(ctx.fail_on, Severity::Low);
    struct Plain;
    impl Doctor for Plain {
        fn name(&self) -> &str {
            "p"
        }
        fn version(&self) -> &str {
            "0"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec!["c"]
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![Box::new(One)]
        }
    }
    struct One;
    impl Check for One {
        fn id(&self) -> &'static str {
            "p.one"
        }
        fn category(&self) -> &'static str {
            "c"
        }
        fn run(&self, _: &Ctx) -> CheckResult {
            CheckResult::findings(vec![finding(
                "p.one",
                "c",
                Severity::High,
                "m",
                Path::new("a"),
                Some(3),
            )])
        }
    }
    assert_eq!(Plain.target(), TargetKind::Dir);
    assert_eq!(Plain.default_fail_on(), Severity::High);
    let env = Plain.scan(&ctx);
    assert_eq!(env.findings[0].extra["check"], "p.one");
    assert_eq!(env.data["checks_run"], 1);
}

#[test]
fn tag_checks_false_leaves_extra_empty() {
    struct Untagged;
    impl Doctor for Untagged {
        fn name(&self) -> &str {
            "u"
        }
        fn version(&self) -> &str {
            "0"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec!["c"]
        }
        fn tag_checks(&self) -> bool {
            false
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            struct C;
            impl Check for C {
                fn id(&self) -> &'static str {
                    "u.c"
                }
                fn category(&self) -> &'static str {
                    "c"
                }
                fn run(&self, _: &Ctx) -> CheckResult {
                    CheckResult::findings(vec![finding(
                        "u.c",
                        "c",
                        Severity::Low,
                        "m",
                        Path::new("a"),
                        None,
                    )])
                }
            }
            vec![Box::new(C)]
        }
    }
    let env = Untagged.scan(&Ctx::detached(Config::default(), Severity::High));
    assert!(env.findings[0].extra.is_empty());
}
