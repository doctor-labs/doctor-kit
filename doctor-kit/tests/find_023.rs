//! 0.2.3 (android-doctor#223): a tool supplies its own `find` predicate (options such as
//! `--name` or `--selinux`); the kit walks the tree and formats the result. Default: text search.
use doctor_kit::doctor_core::Severity;
use doctor_kit::session::{FindFilter, Outcome};
use doctor_kit::{Check, CheckResult, Config, Ctx, Doctor, Session};

struct Emit;
impl Check for Emit {
    fn id(&self) -> &'static str {
        "emit"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, _: &Ctx) -> CheckResult {
        CheckResult::findings(vec![doctor_kit::finding(
            "zz.unique",
            "cat",
            Severity::Low,
            "msg",
            std::path::Path::new("f.txt"),
            Some(1),
        )])
    }
}

struct Plain;
impl Doctor for Plain {
    fn name(&self) -> &str {
        "find-doc"
    }
    fn version(&self) -> &str {
        "0.0.0"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["cat"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![Box::new(Emit)]
    }
}

struct Tool;
impl Doctor for Tool {
    fn name(&self) -> &str {
        "find-doc"
    }
    fn version(&self) -> &str {
        "0.0.0"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["cat"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![Box::new(Emit)]
    }
    fn find_filter(&self, args: &[String]) -> Result<Option<FindFilter>, String> {
        match args
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["--none"] => Ok(Some(Box::new(|_| false))),
            ["--kind", "finding"] => Ok(Some(Box::new(|n| n.kind == "finding"))),
            ["--kind", _] => Ok(None),
            _ => Err("usage: find --kind finding | --none".into()),
        }
    }
}

fn text(d: &dyn Doctor, line: &str) -> Result<String, String> {
    let mut s = Session::new(d, Ctx::detached(Config::default(), Severity::High));
    match s.exec(line)? {
        Outcome::Text(t) => Ok(t),
        Outcome::Quit => Ok(String::new()),
    }
}

#[test]
fn default_find_is_still_text_search() {
    assert_eq!(text(&Plain, "find").unwrap_err(), "usage: find <text>");
    assert!(text(&Plain, "find zz.unique")
        .unwrap()
        .contains("zz.unique"));
}

#[test]
fn tool_predicate_selects_nodes() {
    assert!(text(&Tool, "find --kind finding")
        .unwrap()
        .contains("zz.unique"));
    assert_eq!(text(&Tool, "find --none").unwrap(), "(no matches)");
}

#[test]
fn tool_usage_error_is_returned() {
    assert_eq!(
        text(&Tool, "find --bogus").unwrap_err(),
        "usage: find --kind finding | --none"
    );
}
