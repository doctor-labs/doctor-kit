use doctor_core::{Score, Severity};
use doctor_kit::face::render;
use doctor_kit::{finding, Check, CheckResult, Config, Ctx, Doctor, Face, Scope, Theme};
use std::path::Path;

struct One(Severity);
impl Check for One {
    fn id(&self) -> &'static str {
        "t.one"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, _: &Ctx) -> CheckResult {
        CheckResult::findings(vec![finding(
            "t.one",
            "cat",
            self.0,
            "m\u{1b}[31mx",
            Path::new("a"),
            Some(1),
        )])
    }
}
struct D {
    sev: Severity,
    scored: bool,
}
impl Doctor for D {
    fn name(&self) -> &str {
        "t"
    }
    fn version(&self) -> &str {
        "1"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["cat"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![Box::new(One(self.sev))]
    }
    fn score(&self, f: &[doctor_core::Finding], c: &Ctx) -> Option<Score> {
        if self.scored {
            Some(Score::new(100 - f.len() as u8, "t/1", 0))
        } else {
            let _ = c;
            None
        }
    }
}
fn ctx(cfg: Config) -> Ctx {
    let dir = std::env::temp_dir().join("doctor-kit-test-empty");
    std::fs::create_dir_all(&dir).unwrap();
    Ctx::new(dir, Scope::Full, cfg, Severity::High).unwrap()
}

#[test]
fn default_score_uses_weights_and_sanitizes() {
    let d = D {
        sev: Severity::High,
        scored: true,
    };
    let env = D { scored: false, ..d }.scan(&ctx(Config::default()));
    assert_eq!(env.score.model, "none"); // None degrades, scan still works
    assert_eq!(env.exit_code, 1);
    assert!(!env.findings[0].message.contains('\u{1b}'));
    struct Plain;
    impl Doctor for Plain {
        fn name(&self) -> &str {
            "p"
        }
        fn version(&self) -> &str {
            "1"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec!["cat"]
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![Box::new(One(Severity::High))]
        }
    }
    assert_eq!(Plain.scan(&ctx(Config::default())).score.value, 90);
    let cfg = Config::from_toml("[score]\nhigh = 30").unwrap();
    assert_eq!(Plain.scan(&ctx(cfg)).score.value, 70);
}

#[test]
fn doctor_toml_disables_checks() {
    let cfg = Config::from_toml("[checks]\ndisable = [\"t.one\"]").unwrap();
    let env = D {
        sev: Severity::High,
        scored: true,
    }
    .scan(&ctx(cfg));
    assert!(env.findings.is_empty());
    assert_eq!(env.exit_code, 0);
}

#[test]
fn every_face_and_theme_renders_even_without_score() {
    let env = D {
        sev: Severity::Critical,
        scored: false,
    }
    .scan(&ctx(Config::default()));
    for face in [Face::Plain, Face::Rich, Face::Compact] {
        for name in ["mono", "clinical", "contrast"] {
            let mut t = Theme::by_name(name).unwrap();
            t.enabled = false;
            let out = render(face, &env, &t);
            assert!(
                out.contains("t.one") && !out.contains("/100"),
                "{face:?}/{name}"
            );
        }
    }
}

#[test]
fn rich_box_is_rectangular() {
    let env = D {
        sev: Severity::Low,
        scored: true,
    }
    .scan(&ctx(Config::default()));
    let mut t = Theme::mono();
    t.enabled = false;
    let out = render(Face::Rich, &env, &t);
    let w: Vec<usize> = out.lines().map(|l| l.chars().count()).collect();
    assert!(w.iter().all(|x| *x == w[0]), "{w:?}");
}

#[test]
fn theme_toml_overrides_and_rejects_unknown() {
    let t = Theme::mono()
        .with_toml("base = \"clinical\"\nhigh = \"bold #ff0000\"")
        .unwrap();
    assert!(t.paint("high", "x").contains("\u{1b}["));
    assert!(Theme::mono().with_toml("bogus = \"red\"").is_err());
}

#[test]
fn install_block_is_replaced_in_place() {
    use doctor_kit::install::upsert_block;
    let once = upsert_block("top\n", "x", "old");
    let twice = upsert_block(&format!("{once}bottom\n"), "x", "new");
    assert!(twice.starts_with("top\n") && twice.ends_with("bottom\n"));
    assert!(twice.contains("new") && !twice.contains("old"));
}

#[cfg(feature = "mcp")]
#[test]
fn mcp_rejects_bad_input_without_dying() {
    let d = D {
        sev: Severity::High,
        scored: true,
    };
    let bad = doctor_kit::mcp::process_line(&d, b"{nope").unwrap();
    assert_eq!(bad["error"]["code"], -32700);
    let unk =
        doctor_kit::mcp::process_line(&d, br#"{"jsonrpc":"2.0","id":1,"method":"x"}"#).unwrap();
    assert_eq!(unk["error"]["code"], -32601);
    let call = br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nope"}}"#;
    let r = doctor_kit::mcp::process_line(&d, call).unwrap();
    assert_eq!(r["result"]["isError"], true);
}

struct Boom;
impl Check for Boom {
    fn id(&self) -> &'static str {
        "t.boom"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, _: &Ctx) -> CheckResult {
        panic!("kaboom")
    }
}
struct Mixed;
impl Doctor for Mixed {
    fn name(&self) -> &str {
        "m"
    }
    fn version(&self) -> &str {
        "1"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["cat"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![Box::new(Boom), Box::new(One(Severity::Low))]
    }
}

#[test]
fn a_panicking_check_becomes_a_finding_and_the_scan_continues() {
    let env = Mixed.scan(&ctx(Config::default()));
    let ids: Vec<&str> = env.findings.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids, ["t.boom.panic", "t.one"]);
    assert!(env.findings[0].message.contains("kaboom"));
    assert_eq!(env.exit_code, 1);
    assert_eq!(env.data["panicked"][0], "t.boom");
    // clean scans carry no `panicked` key (envelope bytes unchanged)
    let ok = D {
        sev: Severity::Low,
        scored: true,
    }
    .scan(&ctx(Config::default()));
    assert!(ok.data.get("panicked").is_none());
}

#[test]
fn session_verbs_json_registry_and_completion() {
    use doctor_kit::session::{complete, Outcome};
    use doctor_kit::{Command, Reply, Session};
    struct R;
    impl Doctor for R {
        fn name(&self) -> &str {
            "r"
        }
        fn version(&self) -> &str {
            "1"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec!["cat"]
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![Box::new(One(Severity::High))]
        }
        fn commands(&self) -> Vec<Command> {
            vec![Command {
                name: "write",
                help: "demo",
                flags: &["--force"],
                run: |_, _| Ok(Reply::text("wrote")),
            }]
        }
    }
    let mut s = Session::new(&R, ctx(Config::default()));
    let mut run = |l: &str| match s.exec(l).unwrap() {
        Outcome::Text(t) => t,
        Outcome::Quit => "QUIT".into(),
    };
    assert!(run("ls").contains("cat"));
    run("cd cat");
    assert_eq!(run("pwd"), "/cat");
    assert!(run("cat 1").contains("t.one"));
    assert!(run("find t.one").contains("/cat/1"));
    assert!(run("info").contains("by_severity"));
    assert!(run("decode 1").contains("severity: high"));
    run("cd 1");
    assert!(run("raw").contains("0000")); // hexdump of the finding JSON
    run("cd ..");
    let rec: serde_json::Value = serde_json::from_str(&run("why 1 --json")).unwrap();
    assert_eq!(
        (rec["schema"].as_str(), rec["ok"].as_bool()),
        (Some("doctor-shell/1"), Some(true))
    );
    assert_eq!(rec["data"]["id"], "t.one");
    let bad: serde_json::Value = serde_json::from_str(&run("cd nowhere --json")).unwrap();
    assert_eq!(bad["ok"], false);
    assert_eq!(run("write"), "wrote"); // tool command
    assert!(s.exec("verify").is_err()); // vocabulary word the tool did not register
    let h = s.hints();
    assert_eq!(complete(&h, "ex").1, ["export"]);
    assert!(complete(&h, "wr").1.contains(&"write".to_string()));
    assert_eq!(complete(&h, "export j"), (7, vec!["json".to_string()]));
    assert!(complete(&h, "do x --").1.contains(&"--node".to_string()));
    assert_eq!(complete(&h, "write --f").1, ["--force"]);
    assert_eq!(complete(&h, "cat ").1, ["1"]);
    assert!(matches!(s.exec("quit").unwrap(), Outcome::Quit));
}
