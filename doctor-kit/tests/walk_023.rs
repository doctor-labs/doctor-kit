//! 0.2.3 (#32): the directory walk can include hidden entries and override the skip list, and
//! can report what it skipped. Defaults are the 0.2.2 behaviour.
use doctor_kit::check::{skipped_entries, WalkOptions};
use doctor_kit::doctor_core::Severity;
use doctor_kit::{Check, Config, Ctx, Doctor, Scope};
use std::path::PathBuf;

fn tree(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    for (p, body) in [
        ("a.txt", "a"),
        (".hidden", "h"),
        ("target/t.txt", "t"),
        ("node_modules/n.txt", "n"),
        ("sub/b.txt", "b"),
        ("sub/.env", "e"),
    ] {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, body).unwrap();
    }
    root
}

fn rels(ctx: &Ctx) -> Vec<String> {
    ctx.files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

struct Plain;
impl Doctor for Plain {
    fn name(&self) -> &str {
        "walk-doc"
    }
    fn version(&self) -> &str {
        "0.0.0"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["x"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![]
    }
}

struct Opted;
impl Doctor for Opted {
    fn name(&self) -> &str {
        "walk-doc"
    }
    fn version(&self) -> &str {
        "0.0.0"
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["x"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![]
    }
    fn walk_options(&self) -> WalkOptions {
        WalkOptions::new().report_skipped(true)
    }
}

#[test]
fn default_walk_is_unchanged() {
    let root = tree("walk-default");
    let ctx = Ctx::new(&root, Scope::Full, Config::default(), Severity::High).unwrap();
    assert_eq!(rels(&ctx), vec!["a.txt", "sub/b.txt"]);
    assert_eq!(Plain.walk_options(), WalkOptions::default());
}

#[test]
fn opt_in_includes_hidden_and_overrides_skip_list() {
    let root = tree("walk-opt");
    let opts = WalkOptions::new()
        .include_hidden(true)
        .skip_names(["node_modules"]);
    let ctx = Ctx::with_walk(
        &root,
        Scope::Full,
        None,
        Config::default(),
        Severity::High,
        &opts,
    )
    .unwrap();
    assert_eq!(
        rels(&ctx),
        vec![".hidden", "a.txt", "sub/.env", "sub/b.txt", "target/t.txt"]
    );
}

#[test]
fn skipped_entries_lists_what_the_default_walk_drops() {
    let root = tree("walk-skipped");
    let mut got: Vec<String> = skipped_entries(&root, &WalkOptions::new().report_skipped(true))
        .unwrap()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    got.sort();
    assert_eq!(got, vec![".hidden", "node_modules", "sub/.env", "target"]);
}

#[test]
fn report_skipped_adds_an_info_note_only_when_opted_in() {
    let root = tree("walk-note");
    let ctx = Ctx::new(&root, Scope::Full, Config::default(), Severity::High).unwrap();
    let plain = Plain.scan(&ctx);
    assert!(
        plain.data.get("notes").is_none(),
        "default output must not change"
    );
    let env = Opted.scan(&ctx);
    let notes = env.data["notes"].as_array().expect("notes array");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["level"], "info");
    assert!(notes[0]["text"]
        .as_str()
        .unwrap()
        .starts_with("4 entries skipped"));
}
