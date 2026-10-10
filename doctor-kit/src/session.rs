//! `Session` = opened target + cursor over a [`Node`] tree + findings. The REPL and the TUI are
//! two front-ends over `Session::exec`; neither has logic of its own (all-in-one-cli.md section 2).
//! Every command is `(Session, args) -> Result<Reply>`; `--json` prints the reply as a record.
use crate::check::Ctx;
use crate::doctor::{is_scored, severity_counts, Doctor};
use crate::node::{hex, hexdump, parse_hex, Node};
use doctor_core::{sarif, Envelope, Finding};
use serde_json::{json, Value};

pub enum Outcome {
    Text(String),
    Quit,
}

/// What a command returns: text for people, `data` for `--json`.
pub struct Reply {
    pub text: String,
    pub data: Value,
}

impl Reply {
    pub fn new(text: impl Into<String>, data: Value) -> Reply {
        Reply {
            text: text.into(),
            data,
        }
    }
    /// A reply whose JSON form is just `{"text": ...}`.
    pub fn text(text: impl Into<String>) -> Reply {
        let text = text.into();
        Reply {
            data: json!({"text": text}),
            text,
        }
    }
}

pub type CommandFn = fn(&mut Session<'_>, &[String]) -> Result<Reply, String>;

/// A tool's domain command. Base verbs always win a name clash; free-form names are fine.
/// Reuse the shared vocabulary ([`VOCAB`]) where the concept exists.
#[derive(Clone, Copy)]
pub struct Command {
    pub name: &'static str,
    pub help: &'static str,
    /// Flags offered by tab-completion.
    pub flags: &'static [&'static str],
    pub run: CommandFn,
}

/// The shared verbs, identical across tools.
pub const VERBS: [&str; 13] = [
    "ls", "cd", "pwd", "cat", "info", "find", "decode", "raw", "do", "scan", "why", "export",
    "help",
];
/// Recommended names for tool commands, so the same concept has the same word everywhere.
pub const VOCAB: [&str; 4] = ["verify", "connect", "read", "write"];

const HELP: &str = "ls [sel]            list children
cd <sel|..|/>       move the cursor (name, number, or a/b/c path)
pwd                 show the cursor
cat [sel]           show a node (a finding, or the decoded item)
info [sel]          metadata and actions of a node
find <text>         search the tree
decode [sel]        typed decode of a node
raw [<hex bytes>]   passthrough to the target, or hexdump the node
do [action] [--node sel] [args] [--yes]
                    run a node action; gated actions are dry-run without --yes
scan                re-run the doctor on the target
why [n|id]          explain a finding: remedy and check advice
export json|sarif <path>
help | quit
Every command takes --json (one machine-readable record).";

pub struct Session<'a> {
    doctor: &'a dyn Doctor,
    pub ctx: Ctx,
    pub env: Envelope,
    pub tree: Node,
    /// Child indexes from the root.
    pub cursor: Vec<usize>,
    /// Emit every reply as a JSON record.
    pub json: bool,
    /// True once any command failed (for the exit code of `shell -c`).
    pub failed: bool,
    commands: Vec<Command>,
    /// Name paths already offered to `Doctor::expand` (each is asked once per scan).
    expanded: std::collections::HashSet<Vec<String>>,
}

fn take_flag(a: &mut Vec<String>, f: &str) -> bool {
    let n = a.len();
    a.retain(|x| x != f);
    a.len() != n
}

fn take_opt(a: &mut Vec<String>, f: &str) -> Option<String> {
    let i = a.iter().position(|x| x == f)?;
    let v = (i + 1 < a.len()).then(|| a[i + 1].clone());
    a.drain(i..(i + 2).min(a.len()));
    v
}

fn show(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), String::from)
}

impl<'a> Session<'a> {
    pub fn new(doctor: &'a dyn Doctor, ctx: Ctx) -> Session<'a> {
        let env = crate::face::sanitize_envelope(&doctor.scan(&ctx));
        let tree = doctor.tree(&env, &ctx);
        Session {
            doctor,
            ctx,
            env,
            tree,
            cursor: vec![],
            json: false,
            failed: false,
            commands: doctor.commands(),
            expanded: Default::default(),
        }
    }

    /// Ask [`Doctor::expand`] for the children of the (childless) node at `path`, once.
    fn expand_node(&mut self, path: &[usize]) {
        let names: Vec<String> = {
            let mut n = &self.tree;
            let mut v = vec![];
            for &i in path {
                n = &n.children[i];
                v.push(n.name.clone());
            }
            v
        };
        if !self.node_at(path).children.is_empty() || !self.expanded.insert(names.clone()) {
            return;
        }
        let kids = self
            .doctor
            .expand(&self.env, &self.ctx, &names, self.node_at(path));
        if let Some(kids) = kids {
            let mut n = &mut self.tree;
            for &i in path {
                n = &mut n.children[i];
            }
            n.children = kids;
        }
    }

    /// [`Session::expand_node`] for every node `sel` passes through, the last one included.
    fn expand_along(&mut self, sel: &str) {
        let mut path = if sel.starts_with('/') {
            vec![]
        } else {
            self.cursor.clone()
        };
        self.expand_node(&path);
        for seg in sel.split('/').filter(|s| !s.is_empty() && *s != ".") {
            if seg == ".." {
                path.pop();
                continue;
            }
            let Some(i) = Self::child_index(self.node_at(&path), seg) else {
                return;
            };
            path.push(i);
            self.expand_node(&path);
        }
    }

    pub fn tool(&self) -> &str {
        self.doctor.name()
    }

    pub fn node_at(&self, path: &[usize]) -> &Node {
        path.iter().fold(&self.tree, |n, &i| &n.children[i])
    }

    pub fn here(&self) -> &Node {
        self.node_at(&self.cursor)
    }

    pub fn pwd(&self) -> String {
        let (mut n, mut s) = (&self.tree, String::new());
        for &i in &self.cursor {
            n = &n.children[i];
            s += &format!("/{}", n.name);
        }
        if s.is_empty() {
            "/".into()
        } else {
            s
        }
    }

    /// name, then the finding's 1-based global number, then 1-based position.
    fn child_index(n: &Node, seg: &str) -> Option<usize> {
        n.children.iter().position(|c| c.name == seg).or_else(|| {
            let k = seg.parse::<usize>().ok()?;
            n.children
                .iter()
                .position(|c| c.finding == Some(k.wrapping_sub(1)))
                .or_else(|| (1..=n.children.len()).contains(&k).then(|| k - 1))
        })
    }

    /// Resolve `a/b`, `/a`, `..` against the cursor.
    pub fn resolve(&self, sel: &str) -> Result<Vec<usize>, String> {
        let mut path = if sel.starts_with('/') {
            vec![]
        } else {
            self.cursor.clone()
        };
        for seg in sel.split('/').filter(|s| !s.is_empty() && *s != ".") {
            if seg == ".." {
                path.pop();
                continue;
            }
            let i = Self::child_index(self.node_at(&path), seg)
                .ok_or_else(|| format!("no such node: {seg}"))?;
            path.push(i);
        }
        Ok(path)
    }

    /// The node `sel` names, or the one under the cursor.
    pub fn node(&self, sel: Option<&str>) -> Result<&Node, String> {
        match sel {
            Some(s) => Ok(self.node_at(&self.resolve(s)?)),
            None => Ok(self.here()),
        }
    }

    /// Children at the cursor whose name or summary contains `filter` (case-insensitive).
    pub fn entries(&self, filter: &str) -> Vec<&Node> {
        self.here()
            .children
            .iter()
            .filter(|c| filter.is_empty() || c.matches(filter))
            .collect()
    }

    /// A finding by 1-based number or exact id; falls back to the cursor node's finding.
    pub fn pick_finding(&self, arg: Option<&str>) -> Result<usize, String> {
        match arg {
            Some(a) => a
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=self.env.findings.len()).contains(n))
                .map(|n| n - 1)
                .or_else(|| self.env.findings.iter().position(|f| f.id == a))
                .ok_or_else(|| format!("no such finding: {a}")),
            None => self
                .here()
                .finding
                .ok_or_else(|| "name a finding: its number or id".into()),
        }
    }

    pub fn exec(&mut self, line: &str) -> Result<Outcome, String> {
        let mut words: Vec<String> = line.split_whitespace().map(String::from).collect();
        if words.is_empty() {
            return Ok(Outcome::Text(String::new()));
        }
        let verb = words.remove(0);
        if matches!(verb.as_str(), "quit" | "exit" | "q") {
            return Ok(Outcome::Quit);
        }
        // Lazy trees: load the children of the node we are in and of every node a selector
        // argument walks through (`cd a/b`, `ls a`, `cat a/b/c`), before the command looks.
        self.expand_node(&self.cursor.clone());
        for w in words.iter().filter(|w| !w.starts_with('-')) {
            self.expand_along(w);
        }
        let json = take_flag(&mut words, "--json") || self.json;
        let r = self.dispatch(&verb, words);
        self.failed |= r.is_err();
        if !json {
            return r.map(|r| Outcome::Text(r.text));
        }
        let (ok, data, error) = match r {
            Ok(r) => (true, r.data, Value::Null),
            Err(e) => (false, Value::Null, Value::String(e)),
        };
        let rec = json!({"schema": "doctor-shell/1", "tool": self.env.tool,
            "command": line.trim(), "cwd": self.pwd(), "ok": ok, "data": data, "error": error});
        Ok(Outcome::Text(serde_json::to_string(&rec).unwrap()))
    }

    fn dispatch(&mut self, verb: &str, mut a: Vec<String>) -> Result<Reply, String> {
        let sel = a.first().cloned();
        let sel = sel.as_deref();
        match verb {
            "help" | "?" => Ok(self.help()),
            "pwd" => Ok(Reply::new(self.pwd(), json!({"cwd": self.pwd()}))),
            "ls" => self.ls(sel),
            "cd" => {
                self.cursor = self.resolve(sel.unwrap_or("/"))?;
                Ok(Reply::new("", json!({"cwd": self.pwd()})))
            }
            "cat" => self.cat(sel),
            "info" => self.info(sel),
            "find" => self.find(&a.join(" ")),
            "decode" => {
                let n = self.node(sel)?;
                let t = self.doctor.decode(n).unwrap_or_else(|| default_decode(n));
                Ok(Reply::new(t.clone(), json!({"node": n.name, "text": t})))
            }
            "raw" => self.raw(&a.join(" ")),
            "do" => {
                let yes = take_flag(&mut a, "--yes");
                let on = take_opt(&mut a, "--node");
                self.do_action(on.as_deref(), &a, yes)
            }
            "scan" => self.rescan(),
            "why" => {
                let i = self.pick_finding(sel)?;
                Ok(self.why(i))
            }
            "export" => self.export(&a),
            v => match self.commands.iter().find(|c| c.name == v).copied() {
                Some(c) => (c.run)(self, &a),
                None if VOCAB.contains(&v) => {
                    Err(format!("`{v}` is not supported by {}", self.tool()))
                }
                None => Err(format!("unknown verb `{v}` (try `help`)")),
            },
        }
    }

    fn help(&self) -> Reply {
        let mut t = format!("Shared verbs:\n{HELP}");
        if !self.commands.is_empty() {
            t += &format!("\n\n{} commands:", self.tool());
            for c in &self.commands {
                t += &format!("\n{:<19} {}", c.name, c.help);
            }
        }
        t += &format!(
            "\n\nShared vocabulary for tool commands: {}",
            VOCAB.join(", ")
        );
        let tool: Vec<Value> = self
            .commands
            .iter()
            .map(|c| json!({"name": c.name, "help": c.help}))
            .collect();
        Reply::new(
            t,
            json!({"verbs": VERBS, "vocabulary": VOCAB, "tool_commands": tool}),
        )
    }

    fn ls(&self, sel: Option<&str>) -> Result<Reply, String> {
        let n = self.node(sel)?;
        let text = if n.children.is_empty() {
            "(empty)".to_string()
        } else {
            n.children
                .iter()
                .map(|c| format!("{:<14} {:<9} {}", c.name, c.kind, c.summary))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let data = n.children.iter().map(|c| {
            json!({"name": c.name, "kind": c.kind,
            "summary": c.summary, "children": c.children.len(), "actions": c.actions.len()})
        });
        Ok(Reply::new(text, json!(data.collect::<Vec<_>>())))
    }

    fn cat(&self, sel: Option<&str>) -> Result<Reply, String> {
        let n = self.node(sel)?;
        let text = match n.finding {
            Some(i) => {
                let f = &self.env.findings[i];
                let mut s = format!(
                    "{} [{}] {}\n{}",
                    f.id, f.severity, f.location.reference, f.message
                );
                for e in &f.evidence {
                    s += &format!("\nevidence {}: {}", e.reference, e.value);
                }
                s
            }
            None => {
                let body = self.doctor.decode(n).unwrap_or_else(|| default_decode(n));
                format!("{} ({}) {}\n{body}", n.name, n.kind, n.summary)
            }
        };
        Ok(Reply::new(text, n.to_value()))
    }

    fn info(&self, sel: Option<&str>) -> Result<Reply, String> {
        let n = self.node(sel)?;
        let mut t = format!("{} ({}) {}", n.name, n.kind, n.summary);
        for (k, v) in &n.meta {
            t += &format!("\n{k}: {}", show(v));
        }
        if !n.actions.is_empty() {
            let a: Vec<String> = n
                .actions
                .iter()
                .map(|a| format!("{}{}", a.name, if a.gated { " (gated)" } else { "" }))
                .collect();
            t += &format!("\nactions: {}", a.join(", "));
        }
        t += &format!("\nchildren: {}", n.children.len());
        Ok(Reply::new(t, n.to_value()))
    }

    fn find(&self, q: &str) -> Result<Reply, String> {
        if q.is_empty() {
            return Err("usage: find <text>".into());
        }
        let mut hits = vec![];
        fn walk(n: &Node, path: String, q: &str, out: &mut Vec<(String, Value, String)>) {
            for c in &n.children {
                let p = format!("{path}/{}", c.name);
                if c.matches(q) {
                    let line = format!("{p:<28} {:<9} {}", c.kind, c.summary);
                    out.push((
                        p.clone(),
                        json!({"path": p, "kind": c.kind, "summary": c.summary}),
                        line,
                    ));
                }
                walk(c, p, q, out);
            }
        }
        walk(&self.tree, String::new(), q, &mut hits);
        hits.truncate(100);
        let text = hits
            .iter()
            .map(|h| h.2.clone())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Reply::new(
            if text.is_empty() {
                "(no matches)".into()
            } else {
                text
            },
            json!(hits.into_iter().map(|h| h.1).collect::<Vec<_>>()),
        ))
    }

    fn raw(&self, arg: &str) -> Result<Reply, String> {
        if arg.is_empty() {
            let b = self
                .here()
                .raw
                .as_deref()
                .ok_or("no raw bytes at the cursor")?;
            return Ok(Reply::new(hexdump(b), json!({"hex": hex(b)})));
        }
        let out = self.doctor.raw(&parse_hex(arg)?)?;
        Ok(Reply::new(out.clone(), json!({"response": out})))
    }

    fn do_action(&self, on: Option<&str>, a: &[String], yes: bool) -> Result<Reply, String> {
        let n = self.node(on)?;
        let Some(name) = a.first() else {
            let t = n
                .actions
                .iter()
                .map(|x| {
                    format!(
                        "{:<14} {}{}",
                        x.name,
                        x.help,
                        if x.gated { "  [gated]" } else { "" }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let t = if t.is_empty() {
                "(no actions here)".into()
            } else {
                t
            };
            return Ok(Reply::new(t, json!(n.to_value()["actions"])));
        };
        let act = n
            .actions
            .iter()
            .find(|x| &x.name == name)
            .ok_or_else(|| format!("no action `{name}` on {} (try `do`)", n.name))?;
        let apply = !act.gated || yes;
        let out = self.doctor.act(&self.ctx, n, name, &a[1..], apply)?;
        let text = if apply {
            out.clone()
        } else {
            format!("dry-run (add --yes to apply): {out}")
        };
        Ok(Reply::new(
            text,
            json!({"action": name, "dry_run": !apply, "output": out}),
        ))
    }

    fn rescan(&mut self) -> Result<Reply, String> {
        let ctx = if self.doctor.target() == crate::TargetKind::Detached {
            Ctx::detached(self.ctx.config.clone(), self.ctx.fail_on)
        } else {
            Ctx::with_base(
                &self.ctx.root,
                self.ctx.scope,
                self.ctx.base.clone(),
                self.ctx.config.clone(),
                self.ctx.fail_on,
            )
            .map_err(|e| e.to_string())?
        };
        self.env = crate::face::sanitize_envelope(&self.doctor.scan(&ctx));
        self.tree = self.doctor.tree(&self.env, &ctx);
        self.ctx = ctx;
        self.cursor = vec![];
        self.expanded.clear();
        let sc = if is_scored(&self.env.score) {
            format!(", score {}", self.env.score.value)
        } else {
            String::new()
        };
        Ok(Reply::new(
            format!(
                "scanned {} files{sc}, {} findings. `ls`, `why 1`.",
                self.ctx.files.len(),
                self.env.findings.len()
            ),
            self.env.to_value(),
        ))
    }

    fn why(&self, i: usize) -> Reply {
        let f = &self.env.findings[i];
        let advice = f
            .extra
            .get("check")
            .and_then(Value::as_str)
            .and_then(|c| self.env.data["advice"][c].as_str());
        Reply::new(
            format!(
                "{} ({}): {}\nfix: {}\nadvice: {}",
                f.id,
                f.severity,
                f.message,
                f.remedy.as_deref().unwrap_or("no known fix"),
                advice.unwrap_or("none")
            ),
            json!({"id": f.id, "severity": f.severity, "message": f.message,
                   "remedy": f.remedy, "advice": advice}),
        )
    }

    fn export(&self, args: &[String]) -> Result<Reply, String> {
        let [fmt, path] = args else {
            return Err("usage: export json|sarif <path>".into());
        };
        let v = match fmt.as_str() {
            "json" => self.env.to_value(),
            "sarif" => to_sarif(&self.env),
            _ => return Err("export formats: json, sarif".into()),
        };
        std::fs::write(path, serde_json::to_string_pretty(&v).unwrap() + "\n")
            .map_err(|e| e.to_string())?;
        Ok(Reply::new(
            format!("wrote {path}"),
            json!({"path": path, "format": fmt}),
        ))
    }

    /// Everything tab-completion needs, detached from the session borrow.
    pub fn hints(&self) -> Hints {
        let mut verbs: Vec<String> = VERBS.iter().map(|s| s.to_string()).collect();
        verbs.extend(self.commands.iter().map(|c| c.name.to_string()));
        let mut flags: Vec<(String, Vec<String>)> =
            vec![("do".into(), vec!["--yes".into(), "--node".into()])];
        flags.extend(self.commands.iter().map(|c| {
            (
                c.name.to_string(),
                c.flags.iter().map(|f| f.to_string()).collect(),
            )
        }));
        Hints {
            verbs,
            children: self
                .here()
                .children
                .iter()
                .map(|c| c.name.clone())
                .collect(),
            actions: self.here().actions.iter().map(|a| a.name.clone()).collect(),
            flags,
        }
    }
}

/// Snapshot for completion.
#[derive(Debug, Clone, Default)]
pub struct Hints {
    pub verbs: Vec<String>,
    pub children: Vec<String>,
    pub actions: Vec<String>,
    pub flags: Vec<(String, Vec<String>)>,
}

/// Candidates for the word being typed at the end of `line`: `(word start, candidates)`.
/// Verbs first, then per verb: flags after `-`, `json|sarif` for `export`, actions for `do`,
/// children everywhere else.
pub fn complete(h: &Hints, line: &str) -> (usize, Vec<String>) {
    let start = line.rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let word = &line[start..];
    let toks: Vec<&str> = line[..start].split_whitespace().collect();
    let pool: Vec<String> = match toks.first() {
        None => h.verbs.clone(),
        Some(_) if word.starts_with('-') => {
            let mut f = vec!["--json".to_string()];
            f.extend(
                h.flags
                    .iter()
                    .filter(|(v, _)| v == toks[0])
                    .flat_map(|(_, f)| f.clone()),
            );
            f
        }
        Some(&"export") if toks.len() == 1 => vec!["json".into(), "sarif".into()],
        Some(&"do") if toks.len() == 1 => h.actions.clone(),
        Some(_) => h.children.clone(),
    };
    (
        start,
        pool.into_iter().filter(|c| c.starts_with(word)).collect(),
    )
}

fn default_decode(n: &Node) -> String {
    let mut t: Vec<String> = n
        .meta
        .iter()
        .map(|(k, v)| format!("{k}: {}", show(v)))
        .collect();
    if let Some(b) = &n.raw {
        t.push(hexdump(b));
    }
    if t.is_empty() {
        t.push("(nothing to decode)".into());
    }
    t.join("\n")
}

fn finding_node(i: usize, f: &Finding) -> Node {
    let l = f.location.line.map(|l| format!(":{l}")).unwrap_or_default();
    let mut n = Node::new(
        &(i + 1).to_string(),
        "finding",
        format!(
            "{:<9} {:<22} {}{l}",
            f.severity.as_str(),
            f.id,
            f.location.reference
        ),
    )
    .meta("id", f.id.as_str())
    .meta("severity", f.severity.as_str())
    .meta("category", f.category.as_str())
    .meta(
        "check",
        f.extra.get("check").cloned().unwrap_or(Value::Null),
    )
    .meta("fingerprint", f.fingerprint.as_str())
    .meta("location", format!("{}{l}", f.location.reference))
    .meta("remedy", f.remedy.clone())
    .raw(serde_json::to_vec_pretty(f).unwrap_or_default());
    n.finding = Some(i);
    n
}

/// root -> category -> finding. Tools extend or replace it through `Doctor::tree`.
pub fn default_tree(env: &Envelope, ctx: &Ctx) -> Node {
    let score = if is_scored(&env.score) {
        format!(
            "{}/100 ({}, model {})",
            env.score.value, env.score.label, env.score.model
        )
    } else {
        "n/a".into()
    };
    let mut root = Node::new("root", "root", format!("{} {}", env.tool, env.version))
        .meta("target", ctx.root.display().to_string())
        .meta("files", env.data["files"].clone())
        .meta("checks_run", env.data["checks_run"].clone())
        .meta("skipped", env.data["skipped"].clone())
        .meta("score", score)
        .meta("findings", env.findings.len());
    for c in crate::face::categories_of(env).iter().map(String::as_str) {
        let mine: Vec<(usize, &Finding)> = env
            .findings
            .iter()
            .enumerate()
            .filter(|(_, f)| f.category == c)
            .collect();
        let by: Vec<String> = severity_counts(env)
            .iter()
            .map(|(s, _)| (s, mine.iter().filter(|(_, f)| f.severity == *s).count()))
            .filter(|(_, k)| *k > 0)
            .map(|(s, k)| format!("{k} {s}"))
            .collect();
        let mut cat = Node::new(c, "category", format!("{} findings", mine.len()))
            .meta("by_severity", by.join(", "));
        cat.children = mine.iter().map(|(i, f)| finding_node(*i, f)).collect();
        root.children.push(cat);
    }
    root
}

/// SARIF 2.1.0 via doctor-core's scaffold.
pub fn to_sarif(env: &Envelope) -> Value {
    let mut ids: Vec<&str> = env.findings.iter().map(|f| f.id.as_str()).collect();
    ids.dedup();
    let rules = ids.iter().map(|id| json!({"id": id})).collect();
    let results = env
        .findings
        .iter()
        .map(|f| {
            json!({
                "ruleId": f.id, "level": f.severity.sarif_level(),
                "message": {"text": f.message},
                "locations": [{"physicalLocation": {
                    "artifactLocation": {"uri": f.location.reference},
                    "region": {"startLine": f.location.line.unwrap_or(1)}}}],
                "partialFingerprints": sarif::partial_fingerprints(&f.fingerprint),
            })
        })
        .collect();
    sarif::document(
        &env.tool,
        &env.version,
        "https://github.com/doctor-labs/doctor-kit",
        rules,
        results,
        &env.score,
    )
}
