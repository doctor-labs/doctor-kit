//! `Session` = opened target + cursor + findings. The REPL and the TUI are two front-ends over
//! `Session::exec`; neither has logic of its own (all-in-one-cli.md section 2).
use crate::check::Ctx;
use crate::doctor::{is_scored, severity_counts, Doctor};
use doctor_core::{sarif, Envelope, Finding};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cursor {
    Root,
    Category(String),
    /// Index into `env.findings`.
    Finding(usize),
}

pub enum Outcome {
    Text(String),
    Quit,
}

pub struct Session<'a> {
    doctor: &'a dyn Doctor,
    pub ctx: Ctx,
    pub env: Envelope,
    pub cursor: Cursor,
}

pub const VERBS: [&str; 11] = [
    "ls", "cd", "pwd", "cat", "info", "scan", "why", "export", "help", "quit", "exit",
];

const HELP: &str = "ls                 list children at the cursor (categories, then findings)
cd <name|n|..|/>   move the cursor
pwd                show the cursor
cat [n]            show a finding
info               metadata at the cursor
scan               re-run the doctor on the target
why [n|id]         explain a finding: remedy and check advice
export json|sarif <path>
help | quit";

impl<'a> Session<'a> {
    pub fn new(doctor: &'a dyn Doctor, ctx: Ctx) -> Session<'a> {
        let env = doctor.scan(&ctx);
        Session {
            doctor,
            ctx,
            env,
            cursor: Cursor::Root,
        }
    }

    pub fn categories(&self) -> Vec<String> {
        self.env.data["categories"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_str().map(String::from))
            .collect()
    }

    /// Findings under the cursor's category (or all at Root), as global indexes.
    pub fn visible(&self) -> Vec<usize> {
        let cat = match &self.cursor {
            Cursor::Category(c) => Some(c.as_str()),
            Cursor::Finding(i) => Some(self.env.findings[*i].category.as_str()),
            Cursor::Root => None,
        };
        (0..self.env.findings.len())
            .filter(|&i| cat.is_none_or(|c| self.env.findings[i].category == c))
            .collect()
    }

    pub fn pwd(&self) -> String {
        match &self.cursor {
            Cursor::Root => "/".into(),
            Cursor::Category(c) => format!("/{c}"),
            Cursor::Finding(i) => format!("/{}/{}", self.env.findings[*i].category, i + 1),
        }
    }

    fn line(i: usize, f: &Finding) -> String {
        let l = f.location.line.map(|l| format!(":{l}")).unwrap_or_default();
        format!(
            "{:>3}  {:<9} {:<22} {}{l}",
            i + 1,
            f.severity.as_str(),
            f.id,
            f.location.reference
        )
    }

    /// `n` (1-based global number) or an exact finding id; falls back to the cursor.
    fn pick(&self, arg: Option<&str>) -> Result<usize, String> {
        match (arg, &self.cursor) {
            (Some(a), _) => a
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=self.env.findings.len()).contains(n))
                .map(|n| n - 1)
                .or_else(|| self.env.findings.iter().position(|f| f.id == a))
                .ok_or_else(|| format!("no such finding: {a}")),
            (None, Cursor::Finding(i)) => Ok(*i),
            _ => Err("name a finding: its number or id".into()),
        }
    }

    pub fn exec(&mut self, line: &str) -> Result<Outcome, String> {
        let mut w = line.split_whitespace();
        let Some(verb) = w.next() else {
            return Ok(Outcome::Text(String::new()));
        };
        let args: Vec<&str> = w.collect();
        let text = match verb {
            "quit" | "exit" | "q" => return Ok(Outcome::Quit),
            "help" | "?" => HELP.to_string(),
            "pwd" => self.pwd(),
            "ls" => match &self.cursor {
                Cursor::Root => self
                    .categories()
                    .iter()
                    .map(|c| {
                        format!(
                            "{c}/  ({})",
                            self.env
                                .findings
                                .iter()
                                .filter(|f| &f.category == c)
                                .count()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => self
                    .visible()
                    .iter()
                    .map(|&i| Self::line(i, &self.env.findings[i]))
                    .collect::<Vec<_>>()
                    .join("\n"),
            },
            "cd" => {
                self.cd(args.first().copied().unwrap_or("/"))?;
                String::new()
            }
            "cat" => {
                let f = &self.env.findings[self.pick(args.first().copied())?];
                let mut s = format!(
                    "{} [{}] {}\n{}",
                    f.id, f.severity, f.location.reference, f.message
                );
                for e in &f.evidence {
                    s += &format!("\nevidence {}: {}", e.reference, e.value);
                }
                s
            }
            "info" => self.info(),
            "scan" => {
                let ctx = Ctx::new(
                    &self.ctx.root,
                    self.ctx.scope,
                    self.ctx.config.clone(),
                    self.ctx.fail_on,
                )
                .map_err(|e| e.to_string())?;
                let env = self.doctor.scan(&ctx);
                *self = Session {
                    doctor: self.doctor,
                    ctx,
                    env,
                    cursor: Cursor::Root,
                };
                let sc = if is_scored(&self.env.score) {
                    format!(", score {}", self.env.score.value)
                } else {
                    String::new()
                };
                format!(
                    "scanned {} files{sc}, {} findings. `ls`, `why 1`.",
                    self.ctx.files.len(),
                    self.env.findings.len()
                )
            }
            "why" => self.why(self.pick(args.first().copied())?),
            "export" => self.export(&args)?,
            v => return Err(format!("unknown verb `{v}` (try `help`)")),
        };
        Ok(Outcome::Text(text))
    }

    fn cd(&mut self, sel: &str) -> Result<(), String> {
        let cats = self.categories();
        let cat_of = |s: &Session| match &s.cursor {
            Cursor::Category(c) => Some(c.clone()),
            Cursor::Finding(i) => Some(s.env.findings[*i].category.clone()),
            Cursor::Root => None,
        };
        self.cursor = match sel.trim_end_matches('/') {
            "" | "/" => Cursor::Root,
            ".." => match &self.cursor {
                Cursor::Finding(i) => Cursor::Category(self.env.findings[*i].category.clone()),
                _ => Cursor::Root,
            },
            s if cats.iter().any(|c| c == s) => Cursor::Category(s.to_string()),
            s => {
                let i = self
                    .pick(Some(s))
                    .map_err(|_| format!("no such category or finding: {s}"))?;
                if cat_of(self).is_some_and(|c| c != self.env.findings[i].category) {
                    return Err(format!("finding {s} is not in {}", self.pwd()));
                }
                Cursor::Finding(i)
            }
        };
        Ok(())
    }

    fn info(&self) -> String {
        match &self.cursor {
            Cursor::Root => {
                let e = &self.env;
                let score = if is_scored(&e.score) {
                    format!(
                        "{}/100 ({}, model {})",
                        e.score.value, e.score.label, e.score.model
                    )
                } else {
                    "n/a".into()
                };
                format!("{} {}\ntarget: {}\nfiles: {}  checks run: {}  skipped: {}\nscore: {score}\nfindings: {}", e.tool, e.version, self.ctx.root.display(), e.data["files"], e.data["checks_run"], e.data["skipped"], e.findings.len())
            }
            Cursor::Category(c) => {
                let n = self.visible().len();
                let by: Vec<String> = severity_counts(&self.env)
                    .iter()
                    .map(|(s, _)| {
                        (
                            s,
                            self.visible()
                                .iter()
                                .filter(|&&i| self.env.findings[i].severity == *s)
                                .count(),
                        )
                    })
                    .filter(|(_, k)| *k > 0)
                    .map(|(s, k)| format!("{k} {s}"))
                    .collect();
                format!("category {c}: {n} findings ({})", by.join(", "))
            }
            Cursor::Finding(i) => {
                let f = &self.env.findings[*i];
                format!(
                    "fingerprint: {}\ncheck: {}\ncategory: {}\nseverity: {}\nconfidence: {:?}",
                    f.fingerprint,
                    f.extra.get("check").and_then(Value::as_str).unwrap_or("?"),
                    f.category,
                    f.severity,
                    f.confidence
                )
            }
        }
    }

    fn why(&self, i: usize) -> String {
        let f = &self.env.findings[i];
        let advice = f
            .extra
            .get("check")
            .and_then(Value::as_str)
            .and_then(|c| self.env.data["advice"][c].as_str());
        format!(
            "{} ({}): {}\nfix: {}\nadvice: {}",
            f.id,
            f.severity,
            f.message,
            f.remedy.as_deref().unwrap_or("no known fix"),
            advice.unwrap_or("none")
        )
    }

    fn export(&self, args: &[&str]) -> Result<String, String> {
        let [fmt, path] = args else {
            return Err("usage: export json|sarif <path>".into());
        };
        let v = match *fmt {
            "json" => self.env.to_value(),
            "sarif" => to_sarif(&self.env),
            _ => return Err("export formats: json, sarif".into()),
        };
        std::fs::write(path, serde_json::to_string_pretty(&v).unwrap() + "\n")
            .map_err(|e| e.to_string())?;
        Ok(format!("wrote {path}"))
    }
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
