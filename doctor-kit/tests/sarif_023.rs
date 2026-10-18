//! 0.2.3 (#33): SARIF rules are sorted and de-duplicated; a tool can supply rule metadata.
use doctor_kit::check::{finding, RuleMeta};
use doctor_kit::doctor_core::{Finding, Severity};
use doctor_kit::session::to_sarif_with;
use doctor_kit::{Check, CheckResult, Config, Ctx, Doctor, Scope};
use std::path::Path;

struct Emit;
impl Check for Emit {
    fn id(&self) -> &'static str {
        "emit"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, _: &Ctx) -> CheckResult {
        let f = |id: &str, sev, line| -> Finding {
            finding(id, "cat", sev, "msg", Path::new("f.txt"), Some(line))
        };
        CheckResult::findings(vec![
            f("a.two", Severity::High, 1),
            f("a.one", Severity::Medium, 2),
            f("a.two", Severity::Low, 3),
        ])
    }
}

macro_rules! doc {
    ($name:ident $(, $extra:item)*) => {
        struct $name;
        impl Doctor for $name {
            fn name(&self) -> &str { "sarif-doc" }
            fn version(&self) -> &str { "0.0.0" }
            fn categories(&self) -> Vec<&'static str> { vec!["cat"] }
            fn checks(&self) -> Vec<Box<dyn Check>> { vec![Box::new(Emit)] }
            $($extra)*
        }
    };
}

doc!(Plain);
doc!(
    Meta,
    fn rule_meta(&self, id: &str) -> Option<RuleMeta> {
        (id == "a.one").then(|| {
            RuleMeta::new()
                .name("AOne")
                .short_description("short text")
                .full_description("full text")
                .help_uri("https://example.test/a.one")
                .default_level(Severity::High)
        })
    }
);

fn scan<D: Doctor>(d: &D) -> doctor_kit::doctor_core::Envelope {
    let ctx = Ctx::new(".", Scope::Full, Config::default(), Severity::High).unwrap();
    d.scan(&ctx)
}

#[test]
fn rules_are_sorted_and_unique_by_id() {
    let env = scan(&Plain);
    let v = to_sarif_with(&env, &|_| None);
    let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
    let ids: Vec<&str> = rules.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["a.one", "a.two"]);
    assert_eq!(
        rules[1],
        serde_json::json!({"id": "a.two"}),
        "no metadata: id only"
    );
}

#[test]
fn rule_metadata_is_emitted_when_supplied() {
    let env = scan(&Meta);
    let v = Meta.render_sarif(&env);
    let v: serde_json::Value = serde_json::from_str(&v).unwrap();
    let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
    let one = &rules[0];
    assert_eq!(one["id"], "a.one");
    assert_eq!(one["name"], "AOne");
    assert_eq!(one["shortDescription"]["text"], "short text");
    assert_eq!(one["fullDescription"]["text"], "full text");
    assert_eq!(one["helpUri"], "https://example.test/a.one");
    assert_eq!(
        one["defaultConfiguration"]["level"],
        Severity::High.sarif_level()
    );
    assert_eq!(rules[1], serde_json::json!({"id": "a.two"}));
}
