//! 0.2.1: lazy domain trees through Doctor::expand (image -> dir -> file on demand).
use doctor_core::Envelope;
use doctor_kit::session::Outcome;
use doctor_kit::{Check, Collected, Config, Ctx, Doctor, Node, ScanFailure, Session, TargetKind};
use serde_json::Value;
use std::cell::RefCell;

struct Fw {
    asked: RefCell<Vec<String>>,
}
impl Doctor for Fw {
    fn name(&self) -> &str {
        "fw"
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
    fn target(&self) -> TargetKind {
        TargetKind::Detached
    }
    fn collect(&self, _: &Ctx) -> Result<Collected, ScanFailure> {
        Ok(Collected {
            findings: vec![],
            data: Value::Null,
            gaps: 0,
        })
    }
    fn tree(&self, env: &Envelope, ctx: &Ctx) -> Node {
        let mut t = doctor_kit::session::default_tree(env, ctx);
        t.children
            .push(Node::new("boot.img", "image", "an image, not yet read"));
        t
    }
    fn expand(&self, _: &Envelope, _: &Ctx, path: &[String], _: &Node) -> Option<Vec<Node>> {
        self.asked.borrow_mut().push(path.join("/"));
        match path {
            [img] if img == "boot.img" => Some(vec![Node::new("etc", "dir", "directory")]),
            [img, d] if img == "boot.img" && d == "etc" => {
                Some(vec![
                    Node::new("init.rc", "file", "5 bytes").raw(b"hello".to_vec())
                ])
            }
            _ => None,
        }
    }
}

fn text(s: &mut Session, line: &str) -> String {
    match s.exec(line) {
        Ok(Outcome::Text(t)) => t,
        Ok(Outcome::Quit) => "quit".into(),
        Err(e) => format!("ERR {e}"),
    }
}

#[test]
fn children_load_on_demand_once_and_only_where_asked() {
    let d = Fw {
        asked: RefCell::new(vec![]),
    };
    let mut s = Session::new(
        &d,
        Ctx::detached(Config::default(), doctor_core::Severity::High),
    );
    assert!(d.asked.borrow().is_empty(), "nothing is read at startup");
    assert!(text(&mut s, "ls").contains("boot.img"));
    // Entering the image loads its children; listing shows them.
    assert_eq!(text(&mut s, "cd boot.img"), "");
    assert!(text(&mut s, "ls").contains("etc"));
    // A multi-segment selector walks (and loads) through the intermediate nodes.
    assert!(
        text(&mut s, "cat etc/init.rc").contains("init.rc"),
        "{}",
        text(&mut s, "cat etc/init.rc")
    );
    let raw = text(&mut s, "info etc/init.rc");
    assert!(!raw.starts_with("ERR"), "{raw}");
    assert_eq!(
        d.asked.borrow().iter().filter(|p| *p == "boot.img").count(),
        1,
        "asked once: {:?}",
        d.asked.borrow()
    );
    assert_eq!(
        d.asked
            .borrow()
            .iter()
            .filter(|p| *p == "boot.img/etc")
            .count(),
        1
    );
    // An unknown name loads nothing and fails as before.
    assert!(text(&mut s, "cd nope").starts_with("ERR"));
    // A rescan forgets what was loaded and asks again.
    text(&mut s, "scan");
    assert_eq!(text(&mut s, "pwd"), "/");
    text(&mut s, "cd boot.img");
    assert_eq!(
        d.asked.borrow().iter().filter(|p| *p == "boot.img").count(),
        2
    );
}

#[test]
fn the_default_expand_changes_nothing() {
    struct Plain;
    impl Doctor for Plain {
        fn name(&self) -> &str {
            "p"
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
    assert!(Plain
        .expand(
            &Envelope::new(
                "p",
                "1",
                0,
                doctor_core::Score::new(100, "x/1", 0),
                vec![],
                Value::Null
            ),
            &Ctx::detached(Config::default(), doctor_core::Severity::High),
            &[],
            &Node::new("n", "k", "s"),
        )
        .is_none());
}
