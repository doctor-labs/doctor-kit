//! K7: `fix --from`, `--agent`, `--yolo` through the real binary, with a fake agent on PATH.
#![cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixture");
const AGENT_ENV: [&str; 5] = [
    "CLAUDECODE",
    "CODEX_THREAD_ID",
    "CODEX_SANDBOX",
    "CURSOR_SANDBOX",
    "EXAMPLE_DOCTOR_AGENT",
];

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ed-fix-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A `claude` that records its arguments, one per line, in `args.txt`.
fn fake_agent(dir: &std::path::Path) {
    let f = dir.join("claude");
    std::fs::write(&f, "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n---\\n' \"$a\" >> \"$(dirname \"$0\")/args.txt\"; done\n").unwrap();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn fix(args: &[&str], path: &std::path::Path, env: &[(&str, &str)]) -> std::process::Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_example-doctor"));
    c.args(args)
        .env("PATH", format!("{}:/usr/bin:/bin", path.display()));
    for k in AGENT_ENV {
        c.env_remove(k);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

#[test]
fn from_uses_a_saved_report_without_scanning() {
    let t = tmp("from");
    let rep = t.join("r.json");
    let json = Command::new(env!("CARGO_BIN_EXE_example-doctor"))
        .args(["scan", FX, "--json"])
        .output()
        .unwrap();
    std::fs::write(&rep, &json.stdout).unwrap();
    // -C points nowhere: proves nothing is scanned.
    let o = fix(
        &[
            "fix",
            "secret.aws-key",
            "--from",
            rep.to_str().unwrap(),
            "-C",
            "/nonexistent/x",
        ],
        &t,
        &[],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.contains("secret.aws-key") && !out.contains("hygiene.todo"),
        "{out}"
    );
    std::fs::write(&rep, "[]").unwrap();
    let o = fix(&["fix", "--from", rep.to_str().unwrap()], &t, &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("not a doctor/1 report"));
}

#[test]
fn agent_is_launched_with_the_prompt_as_one_argv_and_yolo_adds_the_bypass() {
    let t = tmp("launch");
    fake_agent(&t);
    let o = fix(
        &["fix", "secret.aws-key", "-C", FX, "--agent", "claude"],
        &t,
        &[],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(o.stdout.is_empty(), "prompt goes to the agent, not stdout");
    let args = std::fs::read_to_string(t.join("args.txt")).unwrap();
    assert_eq!(
        args.matches("\n---\n").count(),
        1,
        "exactly one argument: {args}"
    );
    assert!(args.contains("secret.aws-key"));
    std::fs::remove_file(t.join("args.txt")).unwrap();
    let o = fix(
        &[
            "fix",
            "secret.aws-key",
            "-C",
            FX,
            "--agent",
            "claude",
            "--yolo",
        ],
        &t,
        &[],
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stderr).contains("WARNING"));
    let args = std::fs::read_to_string(t.join("args.txt")).unwrap();
    assert!(
        args.starts_with("--dangerously-skip-permissions\n---\n"),
        "{args}"
    );
}

#[test]
fn inside_an_agent_or_without_the_binary_the_prompt_is_printed() {
    let t = tmp("fallback");
    fake_agent(&t);
    let o = fix(
        &["fix", "secret.aws-key", "-C", FX, "--agent", "claude"],
        &t,
        &[("CLAUDECODE", "1")],
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("secret.aws-key"));
    assert!(String::from_utf8_lossy(&o.stderr).contains("running inside an agent"));
    assert!(
        !t.join("args.txt").exists(),
        "never launched from inside an agent"
    );
    let o = fix(
        &["fix", "secret.aws-key", "-C", FX, "--agent", "codex"],
        &t,
        &[],
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("secret.aws-key"));
    assert!(String::from_utf8_lossy(&o.stderr).contains("codex is not on PATH"));
}

#[test]
fn yolo_needs_an_agent_and_unknown_agents_are_refused() {
    let t = tmp("usage");
    assert_eq!(
        fix(&["fix", "--yolo", "-C", FX], &t, &[]).status.code(),
        Some(2)
    );
    assert_eq!(
        fix(&["fix", "--agent", "gpt", "-C", FX], &t, &[])
            .status
            .code(),
        Some(2)
    );
}
