//! 0.2.3 must keep the 0.2.2 shapes compiling: struct literals and the existing signatures.
use doctor_kit::check::{CheckResult, Ctx};
use doctor_kit::doctor_core::Severity;
use doctor_kit::{Config, Scope};
use std::path::PathBuf;

#[test]
fn ctx_and_check_result_literals_still_compile() {
    let ctx = Ctx {
        root: PathBuf::from("."),
        files: vec![],
        config: Config::default(),
        scope: Scope::Full,
        base: None,
        fail_on: Severity::High,
    };
    let r = CheckResult {
        findings: vec![],
        advice: None,
    };
    assert!(r.findings.is_empty());
    assert_eq!(ctx.files.len(), 0);
}

#[test]
fn ctx_new_and_with_base_keep_their_signatures() {
    let a = Ctx::new(".", Scope::Full, Config::default(), Severity::High).unwrap();
    let b = Ctx::with_base(".", Scope::Full, None, Config::default(), Severity::High).unwrap();
    assert_eq!(a.files, b.files);
}
