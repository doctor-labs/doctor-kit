#![cfg(all(feature = "repl", feature = "tui", feature = "mcp"))]
//! Golden snapshots of example-doctor's observable output (stdout, stderr, exit code, written
//! files). They pin the doctor-kit contract for current users: later kit PRs must not edit the
//! files under `tests/golden/`, only add. Regenerate deliberately with `UPDATE_GOLDEN=1`.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixture");

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ed-golden-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn norm(s: &str, tmp: &Path) -> String {
    let fx = std::fs::canonicalize(FX).unwrap();
    let t = std::fs::canonicalize(tmp).unwrap_or_else(|_| tmp.to_path_buf());
    s.replace(&fx.to_string_lossy().to_string(), "<FX>")
        .replace(FX, "<FX>")
        .replace(&t.to_string_lossy().to_string(), "<TMP>")
        .replace(&tmp.to_string_lossy().to_string(), "<TMP>")
}

fn check(name: &str, got: String) {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&p, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&p)
        .unwrap_or_else(|_| panic!("missing golden {}; run with UPDATE_GOLDEN=1", p.display()));
    assert_eq!(got, want, "golden `{name}` changed");
}

fn run(args: &[&str], stdin: Option<&str>, tmp: &Path) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_example-doctor"))
        .args(args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut si = c.stdin.take().unwrap();
    if let Some(s) = stdin {
        si.write_all(s.as_bytes()).unwrap();
    }
    drop(si);
    let o = c.wait_with_output().unwrap();
    norm(
        &format!(
            "exit: {}\n--- stdout ---\n{}--- stderr ---\n{}",
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        tmp,
    )
}

fn case(name: &str, args: &[&str]) {
    let t = tmp(name);
    check(name, run(args, None, &t));
}

fn tree(root: &Path) -> String {
    fn walk(d: &Path, root: &Path, out: &mut Vec<String>) {
        let mut es: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        es.sort();
        for p in es {
            if p.is_dir() {
                walk(&p, root, out);
            } else {
                out.push(format!(
                    "=== {} ===\n{}",
                    p.strip_prefix(root).unwrap().display(),
                    std::fs::read_to_string(&p).unwrap()
                ));
            }
        }
    }
    let mut v = vec![];
    walk(root, root, &mut v);
    v.join("\n")
}

#[test]
fn scan_json_and_score() {
    case("scan-json", &["scan", FX, "--json"]);
    case("scan-score", &["scan", FX, "--score"]);
    case(
        "scan-fail-on-critical",
        &["scan", FX, "--score", "--fail-on", "critical"],
    );
    case(
        "scan-fail-on-info-json",
        &["scan", FX, "--json", "--fail-on", "info"],
    );
}

#[test]
fn scan_faces() {
    for face in ["rich", "plain", "compact"] {
        for theme in ["mono", "clinical", "contrast"] {
            case(
                &format!("scan-{face}-{theme}"),
                &[
                    "scan", FX, "--face", face, "--theme", theme, "--color", "never",
                ],
            );
        }
    }
    case("scan-headless", &["scan", FX, "--headless"]);
}

#[test]
fn scan_errors_and_usage() {
    case(
        "scan-missing-dir",
        &["scan", "/nonexistent/ed-golden", "--json"],
    );
    case("scan-bad-flag", &["scan", FX, "--nope"]);
    case("scan-json-and-score", &["scan", FX, "--json", "--score"]);
    case("scan-bad-fail-on", &["scan", FX, "--fail-on", "huge"]);
    case("no-subcommand", &[]);
}

#[test]
fn scan_sarif_file() {
    let t = tmp("sarif");
    let out = t.join("out.sarif");
    let o = run(
        &["scan", FX, "--score", "--sarif", out.to_str().unwrap()],
        None,
        &t,
    );
    let file = norm(&std::fs::read_to_string(&out).unwrap(), &t);
    check("scan-sarif", format!("{o}--- sarif file ---\n{file}"));
}

#[test]
fn help_and_version() {
    let t = tmp("help");
    let mut all = String::new();
    for a in [
        &["--help"][..],
        &["--version"],
        &["scan", "--help"],
        &["fix", "--help"],
        &["install", "--help"],
        &["ci", "--help"],
        &["ci", "install", "--help"],
        &["mcp", "--help"],
        &["shell", "--help"],
        &["explore", "--help"],
    ] {
        all += &format!("$ example-doctor {}\n{}\n", a.join(" "), run(a, None, &t));
    }
    check("help", all);
}

#[test]
fn fix_prompts() {
    case("fix-all", &["fix", "-C", FX]);
    case("fix-rule", &["fix", "secret.aws-key", "-C", FX]);
    case("fix-none", &["fix", "no.such.id", "-C", FX]);
}

#[test]
fn install_files() {
    let t = tmp("install");
    let o = run(&["install", "--dir", t.to_str().unwrap()], None, &t);
    check("install", format!("{o}{}", norm(&tree(&t), &t)));
    // idempotent re-run
    let o2 = run(&["install", "--dir", t.to_str().unwrap()], None, &t);
    check("install-again", format!("{o2}{}", norm(&tree(&t), &t)));
    let t = tmp("install-hooks");
    let o = run(
        &[
            "install",
            "--dir",
            t.to_str().unwrap(),
            "--agent",
            "claude",
            "--agent-hooks",
        ],
        None,
        &t,
    );
    check(
        "install-claude-hooks",
        format!("{o}{}", norm(&tree(&t), &t)),
    );
    let t = tmp("install-codex");
    let o = run(
        &["install", "--dir", t.to_str().unwrap(), "--agent", "codex"],
        None,
        &t,
    );
    check("install-codex", format!("{o}{}", norm(&tree(&t), &t)));
}

#[test]
fn ci_install() {
    case("ci-print", &["ci", "install", "--print"]);
    let t = tmp("ci");
    let o = run(&["ci", "install", "--dir", t.to_str().unwrap()], None, &t);
    check("ci-write", format!("{o}{}", norm(&tree(&t), &t)));
}

#[test]
fn mcp_transcript() {
    let t = tmp("mcp");
    let reqs = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#.to_string(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_string(),
        format!(r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"scan","arguments":{{"path":"{FX}"}}}}}}"#),
        format!(r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"fix","arguments":{{"path":"{FX}","id":"secret.aws-key"}}}}}}"#),
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"scan","arguments":{"path":"/nonexistent/ed-golden"}}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":7,"method":"nope"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":8,"method":"ping"}"#.to_string(),
        "".to_string(),
        "{not json".to_string(),
    ];
    check(
        "mcp-transcript",
        run(&["mcp"], Some(&(reqs.join("\n") + "\n")), &t),
    );
}

#[test]
fn shell_script() {
    let t = tmp("shell");
    check(
        "shell-c",
        run(
            &["shell", FX, "-c", "ls; cd secrets; ls; cd ..; ls"],
            None,
            &t,
        ),
    );
    check(
        "shell-c-json",
        run(&["shell", FX, "-c", "ls", "--json"], None, &t),
    );
}
