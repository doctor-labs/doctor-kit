//! Scan progress events (issue #29): the scan loop reports what it is doing to an optional sink.
use doctor_core::Severity;
use doctor_kit::{
    finding, Check, CheckResult, Config, Ctx, Doctor, Progress, ProgressEvent, Scope,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

struct Emits(&'static str, Vec<Severity>);
impl Check for Emits {
    fn id(&self) -> &'static str {
        self.0
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn run(&self, _: &Ctx) -> CheckResult {
        CheckResult::findings(
            self.1
                .iter()
                .map(|s| finding(self.0, "cat", *s, "m", Path::new("a"), Some(1)))
                .collect(),
        )
    }
}

struct Skipped;
impl Check for Skipped {
    fn id(&self) -> &'static str {
        "t.b"
    }
    fn category(&self) -> &'static str {
        "cat"
    }
    fn applies(&self, _: &Ctx) -> bool {
        false
    }
    fn run(&self, _: &Ctx) -> CheckResult {
        unreachable!("a skipped check must not run")
    }
}

struct D;
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
        vec![
            Box::new(Emits("t.a", vec![Severity::High, Severity::Low])),
            Box::new(Skipped),
            Box::new(Emits("t.c", vec![])),
        ]
    }
}

/// Records every event as a short string, in arrival order.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);
impl Progress for Recorder {
    fn event(&self, e: &ProgressEvent) {
        let line = match e {
            ProgressEvent::CheckStarted { id, index, total } => {
                format!("start {id} {index}/{total}")
            }
            ProgressEvent::CheckSkipped { id, index, total } => {
                format!("skip {id} {index}/{total}")
            }
            ProgressEvent::Finding { id, severity, .. } => {
                format!("finding {id} {}", format!("{severity:?}").to_lowercase())
            }
            ProgressEvent::CheckFinished { id, findings } => format!("end {id} {findings}"),
            ProgressEvent::Done { findings } => format!("done {findings}"),
        };
        self.0.lock().unwrap().push(line);
    }
}

fn ctx() -> Ctx {
    let dir = std::env::temp_dir().join("doctor-kit-progress-empty");
    std::fs::create_dir_all(&dir).unwrap();
    Ctx::new(dir, Scope::Full, Config::default(), Severity::High).unwrap()
}

#[test]
fn the_scan_loop_reports_each_check_finding_skip_and_the_end_in_order() {
    let rec = Arc::new(Recorder::default());
    let ctx = ctx().with_progress(rec.clone());
    let env = D.scan(&ctx);
    assert_eq!(env.findings.len(), 2);
    assert_eq!(
        *rec.0.lock().unwrap(),
        [
            "start t.a 1/3",
            "finding t.a high",
            "finding t.a low",
            "end t.a 2",
            "skip t.b 2/3",
            "start t.c 3/3",
            "end t.c 0",
            "done 2",
        ]
    );
}

#[test]
fn a_progress_sink_never_changes_the_scan_result() {
    let plain = D.scan(&ctx());
    let watched = D.scan(&ctx().with_progress(Arc::new(Recorder::default())));
    assert_eq!(
        format!("{:?}", plain.findings),
        format!("{:?}", watched.findings)
    );
    assert_eq!(plain.exit_code, watched.exit_code);
}
