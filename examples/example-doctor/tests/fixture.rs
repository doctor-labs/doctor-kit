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

const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixture");
fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_example-doctor"))
}
fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("example-doctor-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn mcp_lists_tools_and_scan_is_byte_identical_to_the_cli() {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = bin()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let reqs = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#.to_string(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_string(),
        format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"scan","arguments":{{"path":"{FX}"}}}}}}"#
        ),
    ];
    child
        .stdin
        .take()
        .unwrap()
        .write_all((reqs.join("\n") + "\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let lines: Vec<serde_json::Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3); // the notification gets no response
    let names: Vec<&str> = lines[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["scan", "fix"]);
    let cli = bin().args(["scan", FX, "--json"]).output().unwrap();
    assert_eq!(
        lines[2]["result"]["content"][0]["text"].as_str().unwrap(),
        String::from_utf8(cli.stdout).unwrap()
    );
    assert_eq!(lines[2]["result"]["isError"], false);
}

#[test]
fn fix_prints_an_agent_prompt() {
    let all = bin().args(["fix", "-C", FX]).output().unwrap();
    let all = String::from_utf8(all.stdout).unwrap();
    assert!(all.contains("UNTRUSTED SCAN DATA") && all.contains("secret.aws-key"));
    let one = bin()
        .args(["fix", "secret.password", "-C", FX])
        .output()
        .unwrap();
    let one = String::from_utf8(one.stdout).unwrap();
    assert!(one.contains("secret.password") && !one.contains("secret.aws-key"));
    assert!(one.contains("remedy: Move it to a secret store."));
    assert!(one.contains("Verify: run `example-doctor scan`"));
    let bad = bin().args(["fix", "nope", "-C", FX]).output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn install_and_ci_install_write_the_expected_files() {
    let d = tmp("install");
    std::fs::write(d.join("AGENTS.md"), "# Mine\n").unwrap();
    for _ in 0..2 {
        let st = bin()
            .args(["install", "--agent-hooks", "--dir"])
            .arg(&d)
            .status()
            .unwrap();
        assert!(st.success());
    }
    let skill = std::fs::read_to_string(d.join(".claude/skills/example-doctor/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\nname: example-doctor") && skill.contains("scan --json"));
    assert!(d.join(".cursor/rules/example-doctor.mdc").is_file());
    let agents = std::fs::read_to_string(d.join("AGENTS.md")).unwrap();
    assert!(agents.starts_with("# Mine\n"));
    assert_eq!(agents.matches("<!-- example-doctor:start -->").count(), 1);
    let settings = std::fs::read_to_string(d.join(".claude/settings.json")).unwrap();
    assert_eq!(settings.matches("--scope changed").count(), 1);
    assert!(bin()
        .args(["ci", "install", "--dir"])
        .arg(&d)
        .status()
        .unwrap()
        .success());
    let wf = std::fs::read_to_string(d.join(".github/workflows/example-doctor.yml")).unwrap();
    assert!(wf.contains("pull_request") && wf.contains("example-doctor scan . --scope changed"));
    assert!(wf.contains("fetch-depth: 0") && wf.contains("cargo install --locked example-doctor"));
}

#[test]
fn scope_changed_sees_modified_untracked_and_base_diffs() {
    let d = tmp("scope");
    let git = |args: &[&str]| {
        let st = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&d)
            .output()
            .unwrap();
        assert!(st.status.success(), "{args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a.txt"), "TODO one\n").unwrap();
    std::fs::write(d.join("README.md"), "hi\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    git(&["checkout", "-qb", "pr"]);
    std::fs::write(d.join("b.txt"), "TODO two\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "pr"]);
    std::fs::write(d.join("c.txt"), "TODO three\n").unwrap(); // untracked
    let refs = |extra: &[&str]| -> Vec<String> {
        let o = bin()
            .args(["scan", "--json", "--scope", "changed"])
            .arg(&d)
            .args(extra)
            .output()
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        let mut r: Vec<String> = v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["id"] == "hygiene.todo")
            .map(|f| f["location"]["ref"].as_str().unwrap().to_string())
            .collect();
        r.sort();
        r
    };
    assert_eq!(refs(&[]), ["c.txt"]);
    assert_eq!(refs(&["--base", "main"]), ["b.txt", "c.txt"]);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}

fn shell(dir: &std::path::Path, cmds: &str) -> Vec<serde_json::Value> {
    let o = bin()
        .args(["shell", "--json", "-c", cmds])
        .arg(dir)
        .output()
        .unwrap();
    String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn shell_json_walks_the_rich_node_and_gates_the_write() {
    let d = tmp("shell");
    copy_dir(std::path::Path::new(FX), &d);
    let r = shell(
        &d,
        "cd files; ls; cat notes_db.txt; do --node notes_db.txt; frob",
    );
    assert_eq!(r[0]["ok"], true);
    assert_eq!(r[1]["cwd"], "/files");
    assert!(r[1]["data"].as_array().unwrap().len() >= 5);
    assert_eq!(r[2]["data"]["meta"]["path"], "notes/db.txt");
    assert!(r[2]["data"]["raw_hex"].as_str().unwrap().len() > 8);
    assert_eq!(r[4]["ok"], false); // unknown verb is a record, not a crash

    let todo = "src_todo.txt";
    let before = std::fs::read_to_string(d.join("src/todo.txt")).unwrap();
    let dry = shell(&d, &format!("do strip-todos --node files/{todo}"));
    assert_eq!(dry[0]["data"]["dry_run"], true);
    assert_eq!(
        std::fs::read_to_string(d.join("src/todo.txt")).unwrap(),
        before
    );
    let live = shell(&d, &format!("do strip-todos --node files/{todo} --yes"));
    assert_eq!(live[0]["data"]["dry_run"], false);
    let after = std::fs::read_to_string(d.join("src/todo.txt")).unwrap();
    assert!(before.contains("TODO") && !after.contains("TODO"));
    // the tool's own command and the shared-vocabulary stub for one it lacks
    let r = shell(&d, "read files/src_clean.txt; verify");
    assert_eq!(r[0]["ok"], true);
    assert_eq!(r[1]["ok"], false);
}

#[test]
fn scan_writes_sarif_without_changing_stdout() {
    let d = tmp("sarif");
    let p = d.join("out.sarif");
    let with = bin()
        .args(["scan", FX, "--json", "--sarif"])
        .arg(&p)
        .output()
        .unwrap();
    let without = bin().args(["scan", FX, "--json"]).output().unwrap();
    assert_eq!(with.stdout, without.stdout);
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    assert_eq!(v["version"], "2.1.0");
    assert_eq!(v["runs"][0]["results"].as_array().unwrap().len(), 8);
}
