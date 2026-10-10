//! K3: --baseline, exit 3, --sarif guards, through the real binary.
use std::process::Command;

const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixture");
fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_example-doctor"))
}
fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ed-bl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn baseline_round_trip_and_exit_3() {
    let t = tmp("rt");
    let base = t.join("base.json");
    let out = bin().args(["scan", FX, "--json"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "no baseline: findings gate with 1"
    );
    std::fs::write(&base, &out.stdout).unwrap();

    // Same tree against its own baseline: nothing new, exit 0, counts in the envelope.
    let out = bin()
        .args(["scan", FX, "--json", "--baseline", base.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["exit_code"], 0);
    assert_eq!(v["baseline"]["new"], 0);
    assert_eq!(v["baseline"]["fixed"], 0);
    assert!(v["baseline"]["unchanged"].as_u64().unwrap() > 0);

    // A baseline missing one finding makes it NEW: exit 3.
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&base).unwrap()).unwrap();
    doc["findings"].as_array_mut().unwrap().remove(0);
    std::fs::write(&base, doc.to_string()).unwrap();
    let out = bin()
        .args(["scan", FX, "--json", "--baseline", base.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (v["exit_code"].as_u64(), v["baseline"]["new"].as_u64()),
        (Some(3), Some(1))
    );
}

#[test]
fn bad_baseline_fails_before_any_output() {
    let t = tmp("bad");
    let base = t.join("b.json");
    std::fs::write(&base, "{\"nope\":1}").unwrap();
    let out = bin()
        .args(["scan", FX, "--json", "--baseline", base.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a doctor/1 report"));
}

#[test]
fn sarif_and_baseline_must_differ() {
    let t = tmp("same");
    let p = t.join("x.json");
    std::fs::write(&p, "[]").unwrap();
    let out = bin()
        .args([
            "scan",
            FX,
            "--score",
            "--baseline",
            p.to_str().unwrap(),
            "--sarif",
            p.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "[]",
        "baseline untouched"
    );
}

#[test]
fn sarif_write_failure_still_prints_the_report() {
    let out = bin()
        .args([
            "scan",
            FX,
            "--score",
            "--sarif",
            "/nonexistent-dir-ed/x.sarif",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "57");
    assert!(String::from_utf8_lossy(&out.stderr).contains("x.sarif"));
}

#[test]
fn sarif_is_written_atomically_without_leftovers() {
    let t = tmp("atomic");
    let p = t.join("o.sarif");
    let out = bin()
        .args(["scan", FX, "--score", "--sarif", p.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(std::fs::read_to_string(&p)
        .unwrap()
        .contains("\"version\": \"2.1.0\""));
    assert_eq!(
        std::fs::read_dir(&t).unwrap().count(),
        1,
        "no temp file left"
    );
}
