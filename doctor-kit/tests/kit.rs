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
