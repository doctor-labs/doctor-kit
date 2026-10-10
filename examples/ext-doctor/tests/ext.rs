//! K4: the extension API through a real binary.
use std::io::Write;
use std::process::{Command, Stdio};

fn run(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_ext-doctor"))
        .args(args)
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn help_lists_replaced_scan_once_and_the_new_command() {
    let (c, out, _) = run(&["--help"]);
    assert_eq!(c, 0);
    assert_eq!(out.matches("Inspect a subject").count(), 1, "{out}");
    assert!(
        !out.contains("Scan a target and print a report"),
        "kit scan replaced: {out}"
    );
    assert!(out.contains("probe") && out.contains("--profile"), "{out}");
    // Replaced in place: scan stays first, as the kit's was.
    assert!(out.find("scan").unwrap() < out.find("probe").unwrap());
    let (_, scan_help, _) = run(&["scan", "--help"]);
    assert!(
        scan_help.contains("--subject") && scan_help.contains("[default: critical]"),
        "{scan_help}"
    );
    assert!(!scan_help.contains("[PATH]"));
}

#[test]
fn detached_builtins_have_no_directory_arguments() {
    for sub in ["fix", "shell"] {
        let (_, out, _) = run(&[sub, "--help"]);
        assert!(
            !out.contains("[PATH]") && !out.contains("--scope") && !out.contains("--base"),
            "{sub}: {out}"
        );
    }
    let (c, out, _) = run(&["fix"]);
    assert_eq!(c, 0, "fix works without a directory");
    assert!(out.is_empty(), "no findings, nothing to print: {out}");
}

#[test]
fn replaced_scan_uses_the_kit_pipeline_with_compact_json_and_own_default_fail_on() {
    let (c, out, _) = run(&["scan", "--subject", "bad", "--json"]);
    assert_eq!(c, 1, "critical >= the doctor's default fail-on");
    assert_eq!(out.lines().count(), 1, "tool-chosen compact JSON: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["findings"][0]["fingerprint"], "fp-bad");
    assert_eq!(v["data"]["subject"], "bad");
    assert_eq!(v["score"]["model"], "ext/1");
    // low finding: below critical, exit 0; --fail-on low flips it
    assert_eq!(run(&["scan", "--subject", "meh", "--json"]).0, 0);
    assert_eq!(
        run(&["scan", "--subject", "meh", "--json", "--fail-on", "low"]).0,
        1
    );
}

#[test]
fn global_argument_reaches_the_builder() {
    let (c, out, _) = run(&["probe", "--profile", "lab"]);
    assert_eq!((c, out.trim()), (0, "probing with profile lab"));
    let (_, out, _) = run(&["--profile", "x", "probe"]);
    assert_eq!(out.trim(), "probing with profile x");
    assert_eq!(run(&["probe", "--code", "7"]).0, 7, "ExtCommand exit code");
}

#[test]
fn failure_envelope_on_stdout_and_stderr_text() {
    let (c, out, err) = run(&["scan", "--subject", "fail", "--json"]);
    assert_eq!(c, 2);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"]["error"]["kind"], "unreachable");
    assert_eq!(v["data"]["touched"], false);
    assert_eq!(err, "error: subject unreachable\n");
}

#[test]
fn baseline_through_the_replaced_scan_gives_exit_3() {
    let d = std::env::temp_dir().join(format!("ext-bl-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let base = d.join("b.json");
    std::fs::write(&base, run(&["scan", "--subject", "ok", "--json"]).1).unwrap();
    let (c, out, _) = run(&[
        "scan",
        "--subject",
        "bad",
        "--json",
        "--baseline",
        base.to_str().unwrap(),
    ]);
    assert_eq!(c, 3);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["exit_code"], 3);
    assert_eq!(v["baseline"]["new"], 1);
}

#[test]
fn usage_exit_is_the_tools() {
    assert_eq!(run(&["scan", "--bogus"]).0, 2, "scan keeps clap's code");
    assert_eq!(
        run(&["probe", "--bogus"]).0,
        129,
        "tool code for other commands"
    );
    assert_eq!(run(&["--version"]).0, 0);
}

#[cfg(feature = "mcp")]
#[test]
fn detached_mcp_offers_no_directory_properties() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ext-doctor"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let props = &v["result"]["tools"][0]["inputSchema"]["properties"];
    assert!(
        props.get("path").is_none()
            && props.get("scope").is_none()
            && props.get("fail_on").is_some()
    );
}

#[cfg(feature = "repl")]
#[test]
fn shell_runs_without_a_directory() {
    let (c, out, _) = run(&["shell", "-c", "ls"]);
    assert_eq!(c, 0, "{out}");
}
