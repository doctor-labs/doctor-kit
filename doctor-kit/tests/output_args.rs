//! K3: OutputArgs definitions and tolerant parsing.
use clap::{Arg, ArgAction, Command};
use doctor_core::Severity;
use doctor_kit::{Check, Doctor, Face, OutputArgs};

struct D(Severity);
impl Doctor for D {
    fn name(&self) -> &str {
        "o"
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
    fn default_fail_on(&self) -> Severity {
        self.0
    }
}

#[test]
fn standard_args_parse_and_default_to_the_doctors_fail_on() {
    let d = D(Severity::Critical);
    let cmd = Command::new("scan").args(OutputArgs::args(&d));
    let m = cmd
        .clone()
        .try_get_matches_from([
            "scan",
            "--json",
            "--sarif",
            "o.sarif",
            "--baseline",
            "b.json",
            "--face",
            "compact",
        ])
        .unwrap();
    let o = OutputArgs::from_matches(&m, &d);
    assert!(o.json && !o.score && !o.headless);
    assert_eq!(o.face, Face::Compact);
    assert_eq!(o.sarif.unwrap().to_str(), Some("o.sarif"));
    assert_eq!(o.baseline.unwrap().to_str(), Some("b.json"));
    assert_eq!(o.fail_on, Severity::Critical);
    let m = cmd
        .try_get_matches_from(["scan", "--fail-on", "low"])
        .unwrap();
    assert_eq!(OutputArgs::from_matches(&m, &d).fail_on, Severity::Low);
}

#[test]
fn a_command_with_a_subset_of_the_ids_takes_defaults_for_the_rest() {
    let d = D(Severity::Medium);
    let cmd = Command::new("mine")
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue))
        .arg(Arg::new("card").long("card"));
    let m = cmd
        .try_get_matches_from(["mine", "--json", "--card", "x"])
        .unwrap();
    let o = OutputArgs::from_matches(&m, &d);
    assert!(o.json);
    assert_eq!(o.face, Face::Rich);
    assert_eq!(o.fail_on, Severity::Medium);
    assert!(o.sarif.is_none() && o.baseline.is_none() && o.theme_root.is_none());
}

#[test]
fn never_fail_is_a_per_command_switch_off_by_default() {
    let d = D(Severity::High);
    let m = Command::new("x")
        .args(OutputArgs::args(&d))
        .try_get_matches_from(["x"])
        .unwrap();
    let o = OutputArgs::from_matches(&m, &d);
    assert!(!o.never_fail);
    assert!(o.never_fail().never_fail);
}
