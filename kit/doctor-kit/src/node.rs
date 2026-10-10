//! The domain-neutral tree the shell and explorer walk. A tool maps its own nouns onto
//! [`Node`] (a SIM EF, a flow, a partition entry, a finding): children, an open-ended metadata
//! map, optional raw bytes for the hex pane, and actions. The skeleton hardcodes no noun.
use serde_json::{json, Map, Value};

/// Something `do <name>` can run on a node. `gated` actions change the target: dry-run by
/// default, applied only with `--yes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub name: String,
    pub help: String,
    pub gated: bool,
}

impl Action {
    pub fn new(name: &str, help: &str, gated: bool) -> Action {
        Action {
            name: name.into(),
            help: help.into(),
            gated,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Node {
    /// Selector for `cd`/`cat`; whitespace and `/` are replaced so it types as one word.
    pub name: String,
    /// Free-form noun chosen by the tool ("category", "finding", "file", "ef", "flow", ...).
    pub kind: String,
    pub summary: String,
    /// Open-ended string -> value metadata (FCP, access conditions, perms, stats, ...).
    pub meta: Map<String, Value>,
    /// Raw bytes for the hex pane and `decode`.
    pub raw: Option<Vec<u8>>,
    pub actions: Vec<Action>,
    pub children: Vec<Node>,
    /// Index into `Envelope::findings` when this node stands for a finding.
    pub finding: Option<usize>,
}

impl Node {
    pub fn new(name: &str, kind: &str, summary: impl Into<String>) -> Node {
        Node {
            name: name
                .chars()
                .map(|c| {
                    if c.is_whitespace() || c == '/' {
                        '_'
                    } else {
                        c
                    }
                })
                .collect(),
            kind: kind.into(),
            summary: summary.into(),
            ..Node::default()
        }
    }
    pub fn meta(mut self, k: &str, v: impl Into<Value>) -> Node {
        self.meta.insert(k.into(), v.into());
        self
    }
    pub fn raw(mut self, bytes: Vec<u8>) -> Node {
        self.raw = Some(bytes);
        self
    }
    pub fn action(mut self, a: Action) -> Node {
        self.actions.push(a);
        self
    }
    pub fn child(mut self, c: Node) -> Node {
        self.children.push(c);
        self
    }

    /// Case-insensitive match on name, summary or any string metadata value.
    pub fn matches(&self, q: &str) -> bool {
        let q = q.to_lowercase();
        self.name.to_lowercase().contains(&q)
            || self.summary.to_lowercase().contains(&q)
            || self
                .meta
                .values()
                .any(|v| v.as_str().is_some_and(|s| s.to_lowercase().contains(&q)))
    }

    /// Machine form used by `--json`.
    pub fn to_value(&self) -> Value {
        json!({
            "name": self.name, "kind": self.kind, "summary": self.summary,
            "meta": self.meta, "children": self.children.len(),
            "actions": self.actions.iter().map(|a| json!({"name": a.name, "help": a.help, "gated": a.gated})).collect::<Vec<_>>(),
            "raw_hex": self.raw.as_deref().map(hex),
        })
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse `00A4 0004`, `0x3f 00`, `3F00` into bytes.
pub fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let digits: String = s
        .split_whitespace()
        .map(|w| w.trim_start_matches("0x"))
        .collect();
    if !digits.len().is_multiple_of(2) || !digits.is_ascii() {
        return Err(format!("not hex bytes: {s}"));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| format!("not hex bytes: {s}"))
        })
        .collect()
}

/// `0000  48 65 6C ...  |Hello|` rows of 16.
pub fn hexdump(b: &[u8]) -> String {
    b.chunks(16)
        .enumerate()
        .map(|(i, c)| {
            let ascii: String = c
                .iter()
                .map(|&x| {
                    if (0x20..0x7f).contains(&x) {
                        x as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{:04X}  {:<47}  |{ascii}|", i * 16, hex(c))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
