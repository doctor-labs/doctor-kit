//! K8: safe install writes, caller-owned markers, agent_files, ci overwrite policy.
#![cfg(unix)]
use doctor_kit::install::{
    ci_install, ci_install_with, install, plan, put_safe, upsert_block_with, write, Agent,
    AgentFile, FileMode, Overwrite,
};
use doctor_kit::{Check, Doctor};
use std::os::unix::fs::symlink;
use std::path::PathBuf;

struct D;
impl Doctor for D {
    fn name(&self) -> &str {
        "t-doctor"
    }
    fn version(&self) -> &str {
        "1"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec![]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![]
    }
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("kit-inst-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_symlinked_directory_on_the_path_is_refused_and_nothing_is_written_through_it() {
    let (root, outside) = (tmp("dirlink"), tmp("dirlink-out"));
    symlink(&outside, root.join(".claude")).unwrap();
    let e = install(&D, &root, Some(Agent::Claude), false).unwrap_err();
    assert!(e.contains("refusing to write through symlink"), "{e}");
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        0,
        "nothing escaped"
    );
    // the workflow directory too
    symlink(&outside, root.join(".github")).unwrap();
    assert!(ci_install(&D, &root).unwrap_err().contains("symlink"));
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn a_symlinked_target_file_is_refused_for_plain_files_and_blocks() {
    let (root, outside) = (tmp("filelink"), tmp("filelink-out"));
    let victim = outside.join("victim");
    std::fs::write(&victim, "precious").unwrap();
    symlink(&victim, root.join("AGENTS.md")).unwrap();
    assert!(install(&D, &root, Some(Agent::Codex), false)
        .unwrap_err()
        .contains("symlink"));
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
    std::fs::create_dir_all(root.join(".cursor/rules")).unwrap();
    symlink(&victim, root.join(".cursor/rules/t-doctor.mdc")).unwrap();
    assert!(install(&D, &root, Some(Agent::Cursor), false)
        .unwrap_err()
        .contains("symlink"));
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
}

#[test]
fn unsafe_components_are_refused() {
    let root = tmp("comp");
    for bad in [&[".."][..], &["a", ".."], &["/etc", "x"], &["a/b"], &[""]] {
        assert!(
            put_safe(&root, bad, "x", Overwrite::Always).is_err(),
            "{bad:?}"
        );
    }
}

#[test]
fn ci_refuses_a_different_file_accepts_the_same_and_force_replaces() {
    let root = tmp("ci");
    let p = ci_install(&D, &root).unwrap();
    assert_eq!(
        ci_install(&D, &root).unwrap(),
        p,
        "identical content: idempotent"
    );
    std::fs::write(&p, "hand edited").unwrap();
    let e = ci_install(&D, &root).unwrap_err();
    assert!(
        e.contains("exists and differs") && e.contains("--force"),
        "{e}"
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "hand edited");
    ci_install_with(&D, &root, Overwrite::Always).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), D.ci_workflow());
    assert!(
        std::fs::read_dir(p.parent().unwrap()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp")),
        "no temp left"
    );
}

#[test]
fn install_keeps_overwriting_and_merging_like_0_1() {
    let root = tmp("keep");
    std::fs::write(root.join("AGENTS.md"), "# Mine\n\nhello\n").unwrap();
    install(&D, &root, None, false).unwrap();
    let a = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(
        a.starts_with("# Mine\n\nhello\n\n<!-- t-doctor:start -->"),
        "{a}"
    );
    let before = a.clone();
    std::fs::write(root.join(".cursor/rules/t-doctor.mdc"), "stale").unwrap();
    install(&D, &root, None, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
        before,
        "block replaced in place"
    );
    assert_ne!(
        std::fs::read_to_string(root.join(".cursor/rules/t-doctor.mdc")).unwrap(),
        "stale"
    );
}

#[test]
fn agent_files_can_set_bodies_paths_and_markers() {
    struct Own;
    impl Doctor for Own {
        fn name(&self) -> &str {
            "own"
        }
        fn version(&self) -> &str {
            "1"
        }
        fn categories(&self) -> Vec<&'static str> {
            vec![]
        }
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![]
        }
        fn agent_files(&self, a: Agent) -> Vec<AgentFile> {
            match a {
                Agent::Codex | Agent::Opencode => vec![AgentFile {
                    path: "AGENTS.md".into(),
                    body: "BODY".into(),
                    mode: FileMode::Block {
                        begin: "<<own".into(),
                        end: "own>>".into(),
                    },
                }],
                Agent::Claude => vec![AgentFile {
                    path: "x/y/z.md".into(),
                    body: "exact".into(),
                    mode: FileMode::Write,
                }],
                Agent::Cursor => vec![],
            }
        }
    }
    let p = plan(&Own, None);
    assert_eq!(p.len(), 2, "claude + codex, cursor empty");
    let root = tmp("own");
    std::fs::write(root.join("AGENTS.md"), "top\n").unwrap();
    write(&root, &p, Overwrite::Always).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("x/y/z.md")).unwrap(),
        "exact"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
        "top\n\n<<own\nBODY\nown>>\n"
    );
}

#[test]
fn upsert_block_with_replaces_between_custom_markers_only() {
    let t = upsert_block_with("a\n", "[[", "]]", "one");
    assert_eq!(t, "a\n\n[[\none\n]]\n");
    let t2 = upsert_block_with(&format!("{t}tail\n"), "[[", "]]", "two");
    assert_eq!(t2, "a\n\n[[\ntwo\n]]\ntail\n");
}
