//! K8: `install --print-only` and `ci install --force` through the binary.
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_example-doctor"))
}
fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ed-inst-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn print_only_prints_every_file_and_writes_nothing() {
    let t = tmp("print");
    let o = bin()
        .args([
            "install",
            "--print-only",
            "--dir",
            t.to_str().unwrap(),
            "--agent-hooks",
        ])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8_lossy(&o.stdout);
    for p in [
        ".claude/skills/example-doctor/SKILL.md",
        ".cursor/rules/example-doctor.mdc",
        "AGENTS.md",
        ".claude/settings.json",
    ] {
        assert!(out.contains(&format!("=== {p} ===")), "{p}: {out}");
    }
    assert!(out.contains("<!-- example-doctor:start -->"));
    assert_eq!(std::fs::read_dir(&t).unwrap().count(), 0);
}

#[test]
fn ci_install_refuses_differing_content_until_force() {
    let t = tmp("ci");
    let d = t.to_str().unwrap();
    assert_eq!(
        bin()
            .args(["ci", "install", "--dir", d])
            .status()
            .unwrap()
            .code(),
        Some(0)
    );
    assert_eq!(
        bin()
            .args(["ci", "install", "--dir", d])
            .status()
            .unwrap()
            .code(),
        Some(0),
        "same content again"
    );
    let wf = t.join(".github/workflows/example-doctor.yml");
    std::fs::write(&wf, "custom").unwrap();
    let o = bin().args(["ci", "install", "--dir", d]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--force"));
    assert_eq!(std::fs::read_to_string(&wf).unwrap(), "custom");
    assert_eq!(
        bin()
            .args(["ci", "install", "--dir", d, "--force"])
            .status()
            .unwrap()
            .code(),
        Some(0)
    );
    assert_ne!(std::fs::read_to_string(&wf).unwrap(), "custom");
}
