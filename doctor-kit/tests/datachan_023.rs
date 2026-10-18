//! 0.2.3 (#35): a check contributes keyed data into `envelope.data` through `Check::run_data`.
use doctor_kit::check::CheckData;
use doctor_kit::check::WalkOptions;
use doctor_kit::doctor::RESERVED_DATA_KEYS;
use doctor_kit::doctor_core::Severity;
use doctor_kit::{Check, CheckResult, Config, Ctx, Doctor, Scope};
use serde_json::{json, Value};
use std::path::PathBuf;

struct Clash;
impl Check for Clash {
    fn id(&self) -> &'static str {
        "clash"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, c: &Ctx) -> CheckResult {
        self.run_data(c).0
    }
    fn run_data(&self, _: &Ctx) -> (CheckResult, CheckData) {
        let mut d = CheckData::new();
        d.insert("checks_run".into(), json!(999));
        d.insert("files".into(), json!("x"));
        d.insert("notes".into(), json!("no"));
        d.insert("own_key".into(), json!(1));
        (CheckResult::ok(), d)
    }
}

struct Boom;
impl Check for Boom {
    fn id(&self) -> &'static str {
        "boom"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, _: &Ctx) -> CheckResult {
        panic!("boom")
    }
}

struct Drift;
impl Doctor for Drift {
    fn name(&self) -> &str {
        "data-doc"
    }
    fn version(&self) -> &str {
        "0.0.0"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["cat"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![Box::new(Boom)]
    }
    fn walk_options(&self) -> WalkOptions {
        WalkOptions::new().report_skipped(true)
    }
}

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
doc!(WithClash, Quiet, Clash);

fn scan<D: Doctor>(d: &D) -> Value {
    let ctx = Ctx::new(".", Scope::Full, Config::default(), Severity::High).unwrap();
    d.scan(&ctx).data
}

#[test]
fn default_check_adds_no_keys() {
    let data = scan(&Plain);
    let keys: Vec<&str> = data
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
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

#[test]
fn kit_keys_win_and_each_collision_is_noted() {
    let ctx = Ctx::new(".", Scope::Full, Config::default(), Severity::High).unwrap();
    let env = WithClash.scan(&ctx);
    assert_eq!(env.data["checks_run"], json!(2), "kit key unchanged");
    assert_eq!(
        env.data["files"],
        json!(ctx.files.len()),
        "kit key unchanged"
    );
    assert_eq!(env.data["own_key"], json!(1), "a non-kit key still lands");
    let texts: Vec<&str> = env.data["notes"]
        .as_array()
        .expect("notes")
        .iter()
        .map(|n| n["text"].as_str().unwrap())
        .collect();
    for k in ["checks_run", "files", "notes"] {
        let want = format!("check clash data key {k} is reserved by the kit and was ignored");
        assert!(
            texts.contains(&want.as_str()),
            "missing note for {k}: {texts:?}"
        );
    }
    assert_eq!(texts.len(), 3);
}

#[test]
fn reserved_list_matches_the_keys_collect_writes() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("datachan-drift");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(".x"), "x").unwrap();
    let ctx = Ctx::with_walk(
        &root,
        Scope::Full,
        None,
        Config::default(),
        Severity::High,
        &WalkOptions::new().report_skipped(true),
    )
    .unwrap();
    let env = Drift.scan(&ctx);
    let mut written: Vec<&str> = env
        .data
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    written.sort();
    let mut reserved = RESERVED_DATA_KEYS.to_vec();
    reserved.sort();
    assert_eq!(written, reserved);
}
