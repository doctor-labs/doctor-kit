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

#[cfg(all(unix, feature = "signals"))]
#[test]
fn sigint_exits_130_with_the_interrupted_envelope() {
    use std::io::{BufRead, BufReader};
    let mut child = Command::new(env!("CARGO_BIN_EXE_ext-doctor"))
        .arg("wait")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "ready");
    Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    let status = child.wait().unwrap();
    // Exit 130 by the process itself, not death by signal (that would be code() == None).
    assert_eq!(status.code(), Some(130));
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut out, &mut rest).unwrap();
    let v: serde_json::Value = serde_json::from_str(rest.trim()).unwrap();
    assert_eq!(v["data"]["error"]["kind"], "interrupted");
    assert_eq!(v["exit_code"], 130);
}

#[cfg(feature = "mcp")]
mod custom_mcp {
    use super::*;

    /// Send lines, return the parsed stdout lines.
    fn session(lines: &[&str]) -> (Vec<serde_json::Value>, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ext-doctor"))
            .arg("mcp")
            .env("EXT_DOCTOR_MCP", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all((lines.join("\n") + "\n").as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        (
            text.lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect(),
            text,
        )
    }
    fn call(id: u32, name: &str, args: &str) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#
        )
    }

    #[test]
    fn exec_failure_gives_the_tool_everything_to_word_its_error() {
        let (r, _) = session(&[
            &call(1, "describe", "{}"),
            &call(2, "describe", r#"{"hang":true}"#),
        ]);
        assert_eq!(
            r[0]["result"]["content"][0]["text"],
            r#"Exit code=Some(7) signal=None stdout="probing with profile default" stderr="""#
        );
        let t = r[1]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            t.starts_with("Timeout code=None signal=Some(9) stdout=\"ready\""),
            "{t}"
        );
    }

    #[test]
    fn exec_self_error_template() {
        let (r, _) = session(&[
            &call(1, "exit7", "{}"),
            &call(2, "exit7", r#"{"plain":true}"#),
        ]);
        assert_eq!(
            r[0]["result"]["content"][0]["text"], " (exit 7)",
            "template: stderr + exit suffix"
        );
        assert_eq!(
            r[1]["result"]["content"][0]["text"], "exited with code 7",
            "default text"
        );
        assert_eq!(r[0]["result"]["isError"], true);
    }

    #[test]
    fn tools_replace_scan_and_fix_and_wire_texts_are_the_tools() {
        let (r, _) = session(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#,
            "",
            &call(4, "fix", "{}"),
        ]);
        assert_eq!(r[0]["result"]["protocolVersion"], "2025-03-26", "echoed");
        let names: Vec<&str> = r[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["inspect", "exit7", "describe", "hang", "flood"]);
        assert_eq!(r[2]["error"]["message"], "method not found");
        assert_eq!(
            r[3]["error"]["message"], "parse error",
            "blank line is a parse error"
        );
        assert_eq!(r[3]["error"]["code"], -32700);
        assert_eq!(r[4]["result"]["isError"], true);
        assert_eq!(
            r[4]["result"]["content"][0]["text"], "unknown tool fix",
            "fix not reachable"
        );
    }

    #[test]
    fn exec_self_results_errors_timeouts_and_caps() {
        let (r, _) = session(&[
            &call(1, "inspect", r#"{"subject":"bad"}"#),
            &call(2, "inspect", r#"{"subject":"--evil"}"#),
            &call(3, "hang", "{}"),
            &call(4, "flood", "{}"),
        ]);
        // exit 1 (a critical finding) is a result, and the text is the CLI's own stdout
        assert_eq!(r[0]["result"]["isError"], false);
        let env: serde_json::Value =
            serde_json::from_str(r[0]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(env["findings"][0]["fingerprint"], "fp-bad");
        // tool-made error text is verbatim
        assert_eq!(r[1]["result"]["isError"], true);
        assert_eq!(
            r[1]["result"]["content"][0]["text"],
            r#"{"error":"bad subject --evil"}"#
        );
        assert!(r[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("timed out"));
        assert_eq!(r[2]["result"]["isError"], true);
        assert_eq!(
            r[3]["result"]["content"][0]["text"],
            "output exceeded 1000 bytes"
        );
    }
}

#[test]
fn audit_uses_the_tools_report_and_never_fails_on_findings() {
    // The tool's own human report replaces the kit face, findings do not gate (exit 0).
    let (c, out, _) = run(&["audit", "--subject", "bad"]);
    assert_eq!((c, out.as_str()), (0, "AUDIT 1 findings: ext.bad\n"));
    // --json is still the kit's (tool-rendered) envelope; the exit stays 0.
    let (c, out, _) = run(&["audit", "--subject", "bad", "--json"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        (
            v["exit_code"].as_u64(),
            v["findings"].as_array().unwrap().len()
        ),
        (Some(0), 1)
    );
    // Even a baseline never gates a report-only command.
    let d = std::env::temp_dir().join(format!("ext-audit-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let base = d.join("b.json");
    std::fs::write(&base, run(&["scan", "--subject", "ok", "--json"]).1).unwrap();
    assert_eq!(
        run(&[
            "audit",
            "--subject",
            "bad",
            "--baseline",
            base.to_str().unwrap()
        ])
        .0,
        0
    );
    // scan (same pipeline, no never_fail) still gates and keeps the kit face
    assert_eq!(
        run(&[
            "scan",
            "--subject",
            "bad",
            "--baseline",
            base.to_str().unwrap()
        ])
        .0,
        3
    );
    let (_, face, _) = run(&["scan", "--subject", "bad", "--headless"]);
    assert!(!face.starts_with("AUDIT"));
}

#[test]
fn ci_install_overwrite_policy_is_the_tools() {
    let t = std::env::temp_dir().join(format!("ext-ci-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(&t).unwrap();
    let ci = |policy: Option<&str>, extra: &[&str]| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ext-doctor"));
        c.args(["ci", "install", "--dir", t.to_str().unwrap()])
            .args(extra);
        if let Some(p) = policy {
            c.env("EXT_DOCTOR_CI", p);
        }
        c.status().unwrap().code()
    };
    let wf = t.join(".github/workflows/ext-doctor.yml");
    assert_eq!(ci(None, &[]), Some(0));
    assert_eq!(ci(None, &[]), Some(0), "default: identical is a no-op");
    assert_eq!(
        ci(Some("never"), &[]),
        Some(2),
        "never: even identical is refused"
    );
    assert_eq!(
        ci(Some("never"), &["--force"]),
        Some(0),
        "--force overrides any policy"
    );
    std::fs::write(&wf, "custom").unwrap();
    assert_eq!(ci(None, &[]), Some(2), "default: differing is refused");
    assert_eq!(std::fs::read_to_string(&wf).unwrap(), "custom");
    assert_eq!(
        ci(Some("always"), &[]),
        Some(0),
        "always: the old replace-it behaviour"
    );
    assert_ne!(std::fs::read_to_string(&wf).unwrap(), "custom");
}

#[cfg(all(unix, feature = "signals"))]
#[test]
fn interrupted_envelope_shape_is_the_tools() {
    use std::io::{BufRead, BufReader, Read};
    let mut child = Command::new(env!("CARGO_BIN_EXE_ext-doctor"))
        .arg("wait")
        .env("EXT_DOCTOR_INTR", "1")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(130));
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    let v: serde_json::Value = serde_json::from_str(rest.trim()).unwrap();
    assert_eq!(v["data"]["custom"], true, "the tool's shape");
    assert_eq!(
        v["data"]["error"]["kind"], "interrupted",
        "kit default kept under it"
    );
    assert_eq!(v["exit_code"], 130);
}
