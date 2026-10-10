//! A toy doctor: scans a directory of text files. This file is the whole point of doctor-kit.
use doctor_core::{Evidence, Severity};
use doctor_kit::{finding, Action, Check, CheckResult, Command, Ctx, Doctor, Node, Reply, Session};

struct Toy;

impl Doctor for Toy {
    fn name(&self) -> &str {
        "example-doctor"
    }
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn categories(&self) -> Vec<&'static str> {
        vec!["secrets", "hygiene", "size", "docs"]
    }
    fn checks(&self) -> Vec<Box<dyn Check>> {
        vec![
            Box::new(Line {
                id: "secret.aws-key",
                cat: "secrets",
                sev: Severity::Critical,
                hit: aws_key,
                msg: "AWS access key id in source",
                fix: "Revoke the key and load it from the environment.",
            }),
            Box::new(Line {
                id: "secret.password",
                cat: "secrets",
                sev: Severity::High,
                hit: password,
                msg: "Hard-coded password",
                fix: "Move it to a secret store.",
            }),
            Box::new(Line {
                id: "hygiene.todo",
                cat: "hygiene",
                sev: Severity::Low,
                hit: todo,
                msg: "Unresolved TODO/FIXME",
                fix: "Resolve it or file an issue.",
            }),
            Box::new(Large),
            Box::new(NoReadme),
        ]
    }

    /// Findings tree plus one rich `files` branch: the sim-style pattern. Each file node carries
    /// metadata, raw head bytes for the hex pane, and a gated action.
    fn tree(&self, env: &doctor_core::Envelope, ctx: &Ctx) -> Node {
        let mut t = doctor_kit::session::default_tree(env, ctx);
        let mut files = Node::new("files", "dir", format!("{} files", ctx.files.len()));
        for rel in &ctx.files {
            let bytes = std::fs::read(ctx.root.join(rel)).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes);
            let todos = text.lines().filter(|l| todo(l)).count();
            let mut n = Node::new(
                &rel.to_string_lossy(),
                "file",
                format!("{} bytes, {todos} TODO", bytes.len()),
            )
            .meta("path", rel.to_string_lossy().to_string())
            .meta("bytes", bytes.len())
            .meta("lines", text.lines().count())
            .meta("todo_lines", todos)
            .raw(bytes.iter().take(64).copied().collect());
            if todos > 0 {
                n = n.action(Action::new(
                    "strip-todos",
                    "delete TODO/FIXME lines from the file",
                    true,
                ));
            }
            files.children.push(n);
        }
        t.children.push(files);
        t
    }

    fn act(
        &self,
        ctx: &Ctx,
        node: &Node,
        action: &str,
        _args: &[String],
        apply: bool,
    ) -> Result<String, String> {
        let rel = node.meta["path"].as_str().ok_or("not a file")?;
        let path = ctx.root.join(rel);
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        match action {
            "strip-todos" => {
                let (gone, kept): (Vec<&str>, Vec<&str>) = text.lines().partition(|l| todo(l));
                if !apply {
                    return Ok(format!(
                        "would delete {} line(s) from {rel}: {gone:?}",
                        gone.len()
                    ));
                }
                let mut out = kept.join("\n");
                if text.ends_with('\n') {
                    out.push('\n');
                }
                std::fs::write(&path, out).map_err(|e| e.to_string())?;
                Ok(format!(
                    "deleted {} line(s) from {rel}; `scan` to re-check",
                    gone.len()
                ))
            }
            a => Err(format!("unknown action {a}")),
        }
    }

    fn commands(&self) -> Vec<Command> {
        vec![Command {
            name: "read",
            help: "print a file node's text",
            flags: &[],
            run: read,
        }]
    }
}

fn read(s: &mut Session, a: &[String]) -> Result<Reply, String> {
    let rel = s.node(a.first().map(String::as_str))?.meta["path"]
        .as_str()
        .ok_or("not a file node")?
        .to_string();
    let text = std::fs::read_to_string(s.ctx.root.join(&rel)).map_err(|e| e.to_string())?;
    Ok(Reply::new(
        text.clone(),
        serde_json::json!({"path": rel, "text": text}),
    ))
}

fn todo(l: &str) -> bool {
    l.contains("TODO") || l.contains("FIXME")
}
fn aws_key(l: &str) -> bool {
    l.find("AKIA").is_some_and(|i| {
        l[i + 4..]
            .chars()
            .take(16)
            .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            .count()
            == 16
    })
}
fn password(l: &str) -> bool {
    let l = l.to_lowercase();
    l.contains("password") && l.contains("=") && l.contains(['"', '\''])
}

/// One check type, many line-pattern rules.
struct Line {
    id: &'static str,
    cat: &'static str,
    sev: Severity,
    hit: fn(&str) -> bool,
    msg: &'static str,
    fix: &'static str,
}

impl Check for Line {
    fn id(&self) -> &'static str {
        self.id
    }
    fn category(&self) -> &'static str {
        self.cat
    }
    fn applies(&self, ctx: &Ctx) -> bool {
        !ctx.files.is_empty()
    }
    fn run(&self, ctx: &Ctx) -> CheckResult {
        let mut out = vec![];
        for f in &ctx.files {
            for (n, l) in ctx
                .read(f)
                .unwrap_or_default()
                .lines()
                .enumerate()
                .filter(|(_, l)| (self.hit)(l))
            {
                let mut x = finding(self.id, self.cat, self.sev, self.msg, f, Some(n as u32 + 1));
                x.remedy = Some(self.fix.into());
                x.evidence.push(Evidence {
                    reference: "line".into(),
                    value: l.trim().chars().take(12).collect::<String>() + "…",
                });
                out.push(x);
            }
        }
        CheckResult::findings(out).advice(format!(
            "Pattern rule `{}`; disable it in doctor.toml under [checks].",
            self.id
        ))
    }
}

struct Large;
impl Check for Large {
    fn id(&self) -> &'static str {
        "size.large-file"
    }
    fn category(&self) -> &'static str {
        "size"
    }
    fn run(&self, ctx: &Ctx) -> CheckResult {
        let big = ctx
            .files
            .iter()
            .filter(|f| std::fs::metadata(ctx.root.join(f)).is_ok_and(|m| m.len() > 4096));
        CheckResult::findings(
            big.map(|f| {
                let mut x = finding(
                    self.id(),
                    "size",
                    Severity::Medium,
                    "File larger than 4 KiB",
                    f,
                    None,
                );
                x.remedy = Some("Split it or move it out of the repo.".into());
                x
            })
            .collect(),
        )
    }
}

struct NoReadme;
impl Check for NoReadme {
    fn id(&self) -> &'static str {
        "docs.no-readme"
    }
    fn category(&self) -> &'static str {
        "docs"
    }
    fn run(&self, ctx: &Ctx) -> CheckResult {
        if ctx
            .files
            .iter()
            .any(|f| f.to_string_lossy().to_lowercase().starts_with("readme"))
        {
            return CheckResult::ok();
        }
        let mut x = finding(
            self.id(),
            "docs",
            Severity::Info,
            "No README at the root",
            std::path::Path::new("."),
            None,
        );
        x.remedy = Some("Add a README.md.".into());
        CheckResult::findings(vec![x])
    }
}

fn main() -> std::process::ExitCode {
    doctor_kit::run(Toy)
}
