//! 0.2.2: a closed stdout (`tool scan | head`) is exit 2 with a message, never a panic (exit 101);
//! explore without a TTY is a clear error; a failed --sarif write reports exit 2 in the envelope.
#![cfg(unix)]
use std::io::Write;
use std::process::{Command, Stdio};

const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixture");

/// Run the tool with its stdout already closed on the reader's side: the shell waits, so the pipe
/// is closed before the first byte is written (write fails with EPIPE).
fn closed_stdout(args: &[&str], stdin: Option<&str>) -> (Option<i32>, String) {
    let mut c = Command::new("sh")
        .args([
            "-c",
            "sleep 0.3; exec \"$0\" \"$@\"",
            env!("CARGO_BIN_EXE_example-doctor"),
        ])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(c.stdout.take());
    let mut si = c.stdin.take().unwrap();
    if let Some(s) = stdin {
        let _ = si.write_all(s.as_bytes());
    }
    drop(si);
    let o = c.wait_with_output().unwrap();
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn assert_exit_2_no_panic(what: &str, (code, err): (Option<i32>, String)) {
    assert!(!err.contains("panicked"), "{what}: {err}");
    assert_eq!(code, Some(2), "{what}: {err}");
    assert!(err.contains("cannot write to stdout"), "{what}: {err}");
}

#[test]
fn closed_stdout_is_exit_2_for_every_output_path() {
    assert_exit_2_no_panic("scan --json", closed_stdout(&["scan", FX, "--json"], None));
    assert_exit_2_no_panic(
        "scan --score",
        closed_stdout(&["scan", FX, "--score"], None),
    );
    for face in ["rich", "plain", "compact"] {
        assert_exit_2_no_panic(face, closed_stdout(&["scan", FX, "--face", face], None));
    }
    assert_exit_2_no_panic("fix", closed_stdout(&["fix", "-C", FX], None));
    assert_exit_2_no_panic("shell -c", closed_stdout(&["shell", FX, "-c", "ls"], None));
    assert_exit_2_no_panic(
        "ci --print",
        closed_stdout(&["ci", "install", "--print"], None),
    );
    assert_exit_2_no_panic(
        "install --print-only",
        closed_stdout(&["install", "--print-only"], None),
    );
    assert_exit_2_no_panic(
        "mcp",
        closed_stdout(
            &["mcp"],
            Some("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n"),
        ),
    );
}

#[test]
fn a_scan_that_fails_before_output_still_reports_its_own_error_on_a_closed_stdout() {
    let (code, err) = closed_stdout(&["scan", "/nonexistent/x", "--json"], None);
    assert_eq!(code, Some(2));
    assert!(err.contains("No such file"), "{err}");
}

#[test]
fn explore_without_a_tty_is_a_clear_error() {
    let o = Command::new(env!("CARGO_BIN_EXE_example-doctor"))
        .args(["explore", FX])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(
        err.contains("interactive terminal") && !err.contains("panicked"),
        "{err}"
    );
}

#[test]
fn failed_sarif_write_is_reflected_in_the_printed_exit_code() {
    let o = Command::new(env!("CARGO_BIN_EXE_example-doctor"))
        .args([
            "scan",
            FX,
            "--json",
            "--sarif",
            "/nonexistent-dir-ed/x.sarif",
        ])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        v["exit_code"], 2,
        "the envelope says what the process exits with"
    );
    assert!(
        !v["findings"].as_array().unwrap().is_empty(),
        "the report is still printed"
    );
}
