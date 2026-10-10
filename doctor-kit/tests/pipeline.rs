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

#[test]
fn interrupt_failure_shape_and_inert_without_a_signal() {
    use doctor_kit::interrupt;
    let d = Dev {
        fail: false,
        fps: vec![],
    };
    assert!(!interrupt::interrupted());
    assert!(interrupt::check(&d).is_ok());
    let f = interrupt::failure(&d);
    assert_eq!((f.exit, f.stderr.as_str()), (130, "error: interrupted\n"));
    let e = f.envelope.unwrap();
    assert_eq!(
        (e.exit_code, e.data["error"]["kind"].as_str()),
        (130, Some("interrupted"))
    );
}

#[test]
fn faces_group_by_finding_category_when_data_has_no_categories() {
    use doctor_kit::face::{render, Face};
    use doctor_kit::Theme;
    let mut a = fnd("r.a", "1", Severity::High);
    a.category = "net".into();
    let mut b = fnd("r.b", "2", Severity::Low);
    b.category = "disk".into();
    let mut c = fnd("r.c", "3", Severity::Low);
    c.category = "net".into();
    let mk = |data: Value| {
        doctor_core::Envelope::new(
            "t",
            "1",
            0,
            Score::new(90, "t/1", 0),
            vec![a.clone(), b.clone(), c.clone()],
            data,
        )
    };
    let mut t = Theme::default();
    t.enabled = false;
    for face in [Face::Rich, Face::Plain, Face::Compact] {
        let free = render(face, &mk(json!({"mine": true})), &t);
        assert!(
            free.contains("r.a") && free.contains("r.b"),
            "{face:?}: {free}"
        );
    }
    let rich = render(Face::Rich, &mk(json!({"mine": true})), &t);
    let (net, disk) = (rich.find("net").unwrap(), rich.find("disk").unwrap());
    assert!(net < disk, "first-seen order, grouped: {rich}");
    assert_eq!(
        rich.matches("net").count(),
        1,
        "one heading for both net findings"
    );
    // A declared list still wins, in its own order.
    let declared = render(Face::Rich, &mk(json!({"categories": ["disk", "net"]})), &t);
    assert!(declared.find("disk").unwrap() < declared.find("net").unwrap());
}

fn hostile() -> doctor_core::Envelope {
    let evil = "a\u{1b}[31mRED\u{1b}]8;;http://x\u{7}link\u{202e}rtl\u{200b}z\u{85}\r\nnext";
    let mut f = fnd("id\u{1b}[2J", "fp-1", Severity::High);
    f.category = "cat\u{1b}[0m".into();
    f.message = evil.into();
    f.location = Location::new(LocationKind::File, evil);
    f.remedy = Some(evil.into());
    f.evidence = vec![doctor_core::Evidence {
        reference: evil.into(),
        value: evil.into(),
    }];
    doctor_core::Envelope::new("t", "1", 1, Score::new(50, "t/1", 0), vec![f], json!({}))
}

fn assert_clean(s: &str, what: &str) {
    for bad in ['\u{1b}', '\u{7}', '\u{202e}', '\u{200b}', '\u{85}', '\r'] {
        assert!(!s.contains(bad), "{what}: {bad:?} survived in {s:?}");
    }
}

#[test]
fn faces_strip_hostile_control_characters_from_finding_text() {
    use doctor_kit::face::{render, Face};
    let mut t = doctor_kit::Theme::default();
    t.enabled = false; // no styling of our own: any ESC in the output came from the finding
    for face in [Face::Plain, Face::Rich, Face::Compact] {
        let out = render(face, &hostile(), &t);
        assert_clean(&out, &format!("{face:?}"));
        assert!(out.contains("RED"), "text kept: {out}");
    }
    // Styled output keeps only the theme's own escapes: strip them and nothing hostile is left.
    let styled = render(Face::Rich, &hostile(), &doctor_kit::Theme::default());
    assert!(!styled.contains("\u{202e}") && !styled.contains("\u{7}"));
}

#[test]
fn shell_replies_and_the_session_tree_are_sanitized_too() {
    use doctor_kit::Session;
    struct H;
    impl Doctor for H {
        fn name(&self) -> &str {
            "h"
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
        fn target(&self) -> TargetKind {
            TargetKind::Detached
        }
        fn collect(&self, _: &Ctx) -> Result<Collected, ScanFailure> {
            Ok(Collected {
                findings: hostile().findings,
                data: json!({}),
                gaps: 0,
            })
        }
    }
    let d = H;
    let mut s = Session::new(&d, Ctx::detached(Config::default(), Severity::High));
    for line in [
        "ls",
        "cat 1",
        "why 1",
        "info",
        "find RED",
        "ls --json",
        "cat 1 --json",
    ] {
        let text = match s.exec(line) {
            Ok(doctor_kit::session::Outcome::Text(t)) => t,
            Ok(_) => String::new(),
            Err(e) => e,
        };
        assert_clean(
            &text.replace("\\u001b", "\u{1b}").replace("\\r", "\r"),
            line,
        );
    }
}

mod notes {
    use super::*;
    use doctor_kit::face::{notes_of, render_with_notes};
    use doctor_kit::session::Outcome;
    use doctor_kit::{Face, Note, NoteLevel, Session, Theme};

    fn env(gaps: usize, data: Value) -> doctor_core::Envelope {
        doctor_core::Envelope::new(
            "t",
            "1",
            0,
            Score::new(80, "t/1", gaps),
            vec![fnd("r.a", "1", Severity::Low)],
            data,
        )
    }

    #[test]
    fn default_notes_are_the_banner_then_data_notes_with_levels() {
        assert!(notes_of(&env(0, json!({}))).is_empty(), "no gaps, no notes");
        let n = notes_of(&env(
            3,
            json!({"notes": ["plain string", {"level": "error", "text": "bad"}, {"level": "info", "text": "fyi"}, {"text": 5}, 7]}),
        ));
        let got: Vec<(NoteLevel, &str)> = n.iter().map(|n| (n.level, n.text.as_str())).collect();
        assert_eq!(got[0].0, NoteLevel::Warn);
        assert!(got[0].1.starts_with("INCOMPLETE: 3 coverage gap(s)"));
        assert_eq!(
            got[1..],
            [
                (NoteLevel::Warn, "plain string"),
                (NoteLevel::Error, "bad"),
                (NoteLevel::Info, "fyi")
            ]
        );
    }

    #[test]
    fn every_face_puts_sanitized_notes_before_the_header_and_findings() {
        let notes = vec![
            Note {
                level: NoteLevel::Error,
                text: "TRUNCATED: walk\u{1b}[31m did not see the whole card\u{202e}".into(),
            },
            Note {
                level: NoteLevel::Info,
                text: "TAR stopped".into(),
            },
        ];
        let mut t = Theme::default();
        t.enabled = false;
        for face in [Face::Plain, Face::Compact, Face::Rich] {
            let out = render_with_notes(face, &env(0, json!({})), &t, &notes);
            assert_clean(&out, &format!("{face:?}"));
            let (a, b, f) = (
                out.find("ERROR: TRUNCATED: walk[31m did not see the whole card")
                    .or_else(|| out.find("ERROR: TRUNCATED: walk"))
                    .unwrap(),
                out.find("NOTE: TAR stopped").unwrap(),
                out.find("r.a").unwrap(),
            );
            assert!(a < b && b < f, "{face:?}: {out}");
        }
        // No notes: identical to the render without the hook.
        let plain = render_with_notes(Face::Plain, &env(0, json!({})), &t, &[]);
        assert_eq!(
            plain,
            doctor_kit::face::render(Face::Plain, &env(0, json!({})), &t)
        );
    }

    struct Gappy;
    impl Doctor for Gappy {
        fn name(&self) -> &str {
            "g"
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
        fn target(&self) -> TargetKind {
            TargetKind::Detached
        }
        fn collect(&self, _: &Ctx) -> Result<Collected, ScanFailure> {
            Ok(Collected {
                findings: vec![],
                data: json!({"notes": ["TRUNCATED: partial"]}),
                gaps: 1,
            })
        }
        fn score_collected(&self, c: &Collected, _: &Ctx) -> Option<Score> {
            Some(Score::new(90, "g/1", c.gaps))
        }
    }

    #[test]
    fn shell_shows_notes_at_the_root_and_in_json_but_not_below() {
        let d = Gappy;
        let mut s = Session::new(&d, Ctx::detached(Config::default(), Severity::High));
        let t = |s: &mut Session, l: &str| match s.exec(l).unwrap() {
            Outcome::Text(t) => t,
            Outcome::Quit => String::new(),
        };
        let ls = t(&mut s, "ls");
        assert!(
            ls.starts_with("WARNING: INCOMPLETE: 1 coverage gap(s)"),
            "{ls}"
        );
        assert!(ls.contains("WARNING: TRUNCATED: partial"));
        let rec: Value = serde_json::from_str(&t(&mut s, "info --json")).unwrap();
        assert_eq!(rec["data"]["notes"][1]["text"], "TRUNCATED: partial");
        assert_eq!(rec["data"]["notes"][0]["level"], "warn");
        // a command about one node does not repeat the banner
        assert!(!t(&mut s, "pwd").contains("WARNING"));
        // after a rescan the notes are recomputed
        assert!(t(&mut s, "scan").contains("TRUNCATED: partial"));
    }

    #[test]
    fn a_tool_that_overrides_notes_owns_them() {
        struct Quiet;
        impl Doctor for Quiet {
            fn name(&self) -> &str {
                "q"
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
            fn target(&self) -> TargetKind {
                TargetKind::Detached
            }
            fn collect(&self, _: &Ctx) -> Result<Collected, ScanFailure> {
                Ok(Collected {
                    findings: vec![],
                    data: json!({}),
                    gaps: 4,
                })
            }
            fn score_collected(&self, c: &Collected, _: &Ctx) -> Option<Score> {
                Some(Score::new(90, "q/1", c.gaps))
            }
            fn notes(&self, _: &doctor_core::Envelope) -> Vec<Note> {
                vec![Note {
                    level: NoteLevel::Info,
                    text: "mine".into(),
                }]
            }
        }
        let d = Quiet;
        let mut s = Session::new(&d, Ctx::detached(Config::default(), Severity::High));
        let Outcome::Text(ls) = s.exec("ls").unwrap() else {
            panic!()
        };
        assert!(
            ls.starts_with("NOTE: mine") && !ls.contains("INCOMPLETE"),
            "{ls}"
        );
    }
}
