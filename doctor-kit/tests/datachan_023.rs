//! 0.2.3 (#35): a check contributes keyed data into `envelope.data` through `Check::run_data`.
use doctor_kit::{Check, CheckResult, Config, Ctx, Doctor, Scope};
use doctor_kit::doctor_core::Severity;
use doctor_kit::check::CheckData;
use serde_json::{json, Value};

struct Quiet;
impl Check for Quiet {
    fn id(&self) -> &'static str {
        "quiet"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, _: &Ctx) -> CheckResult {
        CheckResult::ok()
    }
}

struct Map1;
impl Check for Map1 {
    fn id(&self) -> &'static str {
        "map1"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, c: &Ctx) -> CheckResult {
        self.run_data(c).0
    }
    fn run_data(&self, _: &Ctx) -> (CheckResult, CheckData) {
        let mut d = CheckData::new();
        d.insert("file_map".into(), json!([{"off": 0, "len": 4}]));
        d.insert("shared".into(), json!("first"));
        (CheckResult::ok(), d)
    }
}

struct Map2;
impl Check for Map2 {
    fn id(&self) -> &'static str {
        "map2"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, c: &Ctx) -> CheckResult {
        self.run_data(c).0
    }
    fn run_data(&self, _: &Ctx) -> (CheckResult, CheckData) {
        let mut d = CheckData::new();
        d.insert("shared".into(), json!("second"));
        (CheckResult::ok(), d)
    }
}

macro_rules! doc {
    ($name:ident, $($c:expr),*) => {
        struct $name;
        impl Doctor for $name {
            fn name(&self) -> &str { "data-doc" }
            fn version(&self) -> &str { "0.0.0" }
            fn categories(&self) -> Vec<&'static str> { vec!["cat"] }
            fn checks(&self) -> Vec<Box<dyn Check>> { vec![$(Box::new($c)),*] }
        }
    };
}
doc!(Plain, Quiet);
doc!(WithData, Quiet, Map1, Map2);

fn scan<D: Doctor>(d: &D) -> Value {
    let ctx = Ctx::new(".", Scope::Full, Config::default(), Severity::High).unwrap();
    d.scan(&ctx).data
}

#[test]
fn default_check_adds_no_keys() {
    let data = scan(&Plain);
    let keys: Vec<&str> = data.as_object().unwrap().keys().map(String::as_str).collect();
    assert!(!keys.contains(&"file_map"));
    assert!(keys.contains(&"checks_run"));
}

#[test]
fn keyed_data_merges_later_check_replaces_same_key() {
    let data = scan(&WithData);
    assert_eq!(data["file_map"], json!([{"off": 0, "len": 4}]));
    assert_eq!(data["shared"], "second");
    assert_eq!(data["checks_run"], 3);
}
