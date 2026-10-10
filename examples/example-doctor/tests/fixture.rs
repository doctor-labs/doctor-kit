use std::process::Command;

#[test]
fn fixture_scores_and_fails_on_high() {
    let bin = env!("CARGO_BIN_EXE_example-doctor");
    let fx = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixture");
    let out = Command::new(bin)
        .args(["scan", fx, "--score"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "57");
    assert_eq!(out.status.code(), Some(1));
    let json = Command::new(bin)
        .args(["scan", fx, "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["schema"], "doctor/1");
    assert_eq!(v["findings"].as_array().unwrap().len(), 8);
}
