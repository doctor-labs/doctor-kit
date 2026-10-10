//! Faces: renderers over a doctor/1 `Envelope`. Faces read theme tokens only.
use crate::doctor::{is_scored, severity_counts};
use crate::theme::Theme;
use doctor_core::{Envelope, Finding, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Face {
    /// Minimalist text, one line per finding.
    Plain,
    /// Boxed report with a score gauge, grouped by category.
    Rich,
    /// One-line summary plus a findings table.
    Compact,
}

/// Categories to group by: `data.categories` when it is an array (the declared display order),
/// else those of the findings in order of first appearance, so a doctor with free-form `data`
/// still gets grouped output.
pub fn categories_of(env: &Envelope) -> Vec<String> {
    match env.data["categories"].as_array() {
        Some(a) => a
            .iter()
            .filter_map(|c| c.as_str().map(String::from))
            .collect(),
        None => {
            let mut v: Vec<String> = vec![];
            for f in &env.findings {
                if !v.contains(&f.category) {
                    v.push(f.category.clone());
                }
            }
            v
        }
    }
}

pub fn render(face: Face, env: &Envelope, theme: &Theme) -> String {
    match face {
        Face::Plain => plain(env, theme),
        Face::Rich => rich(env, theme),
        Face::Compact => compact(env, theme),
    }
}

/// Text built twice in lockstep: `plain` for width math, `styled` for output.
#[derive(Default)]
struct Row {
    plain: String,
    styled: String,
}

impl Row {
    fn new() -> Row {
        Row::default()
    }
    fn add(mut self, theme: &Theme, token: &str, text: &str) -> Row {
        self.plain.push_str(text);
        self.styled.push_str(&theme.paint(token, text));
        self
    }
    fn raw(self, theme: &Theme, text: &str) -> Row {
        self.add(theme, "", text)
    }
    fn width(&self) -> usize {
        self.plain.chars().count()
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn loc(f: &Finding) -> String {
    match f.location.line {
        Some(l) => format!("{}:{l}", f.location.reference),
        None => f.location.reference.clone(),
    }
}

fn score_text(env: &Envelope) -> Option<String> {
    is_scored(&env.score).then(|| format!("{}/100 {}", env.score.value, env.score.label))
}

fn counts_row(env: &Envelope, theme: &Theme) -> Row {
    let mut r = Row::new();
    let nz: Vec<_> = severity_counts(env)
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .collect();
    if nz.is_empty() {
        return r.add(theme, "ok", "no findings");
    }
    for (i, (s, n)) in nz.iter().enumerate() {
        if i > 0 {
            r = r.add(theme, "dim", " · ");
        }
        r = r.add(theme, s.as_str(), &format!("{n} {s}"));
    }
    r
}

fn sev_pad(s: Severity) -> String {
    format!("{:<9}", s.as_str())
}

// ---- A: plain -------------------------------------------------------------------------------

fn plain(env: &Envelope, t: &Theme) -> String {
    let mut out = Row::new().add(t, "heading", &format!("{} {}", env.tool, env.version));
    if let Some(s) = score_text(env) {
        out = out
            .add(t, "dim", "  score ")
            .add(t, Theme::score_token(env.score.value), &s);
    }
    let mut s = out.styled + "\n";
    let w_loc = env
        .findings
        .iter()
        .map(|f| loc(f).chars().count())
        .max()
        .unwrap_or(0);
    for f in &env.findings {
        s += &Row::new()
            .add(t, f.severity.as_str(), &sev_pad(f.severity))
            .add(t, "accent", &format!(" {:<22}", f.id))
            .raw(t, &format!(" {:<w_loc$}", loc(f)))
            .add(t, "dim", &format!("  {}", f.message))
            .styled;
        s.push('\n');
    }
    s + &counts_row(env, t).styled + "\n"
}

// ---- B: rich --------------------------------------------------------------------------------

const INNER: usize = 76;

fn boxed(t: &Theme, rows: Vec<Option<Row>>, title: &str) -> String {
    let label = format!(" {title} ");
    let rest = "─".repeat(INNER + 1 - label.chars().count());
    let mut s = t.paint("dim", "╭─")
        + &t.paint("heading", &label)
        + &t.paint("dim", &format!("{rest}╮"))
        + "\n";
    for r in rows {
        match r {
            None => s += &(t.paint("dim", &format!("├{}┤", "─".repeat(INNER + 2))) + "\n"),
            Some(r) => {
                let pad = INNER.saturating_sub(r.width());
                s += &(t.paint("dim", "│ ")
                    + &r.styled
                    + &" ".repeat(pad)
                    + &t.paint("dim", " │")
                    + "\n");
            }
        }
    }
    s + &t.paint("dim", &format!("╰{}╯", "─".repeat(INNER + 2))) + "\n"
}

fn gauge(env: &Envelope, t: &Theme) -> Row {
    const W: usize = 30;
    let tok = Theme::score_token(env.score.value);
    let filled = env.score.value as usize * W / 100;
    Row::new()
        .add(t, "dim", "Score  ")
        .add(t, tok, &"█".repeat(filled))
        .add(t, "dim", &"░".repeat(W - filled))
        .add(
            t,
            tok,
            &format!("  {}/100  {}", env.score.value, env.score.label),
        )
}

fn rich(env: &Envelope, t: &Theme) -> String {
    let mut rows = vec![];
    rows.push(Some(if is_scored(&env.score) {
        gauge(env, t)
    } else {
        Row::new().add(t, "dim", "Score  n/a (this doctor does not score)")
    }));
    rows.push(Some(counts_row(env, t)));
    for c in categories_of(env) {
        let fs: Vec<_> = env.findings.iter().filter(|f| f.category == c).collect();
        if fs.is_empty() {
            continue;
        }
        rows.push(None);
        rows.push(Some(Row::new().add(t, "heading", &c).add(
            t,
            "dim",
            &format!(" ({})", fs.len()),
        )));
        for f in fs {
            rows.push(Some(
                Row::new()
                    .add(
                        t,
                        f.severity.as_str(),
                        &format!("● {}", sev_pad(f.severity)),
                    )
                    .add(t, "accent", &f.id)
                    .add(t, "dim", "  ")
                    .raw(t, &clip(&loc(f), INNER - 12 - f.id.chars().count())),
            ));
            rows.push(Some(
                Row::new()
                    .raw(t, "    ")
                    .raw(t, &clip(&f.message, INNER - 4)),
            ));
            if let Some(r) = &f.remedy {
                rows.push(Some(Row::new().raw(t, "    ").add(t, "ok", "→ ").add(
                    t,
                    "dim",
                    &clip(r, INNER - 6),
                )));
            }
        }
    }
    boxed(t, rows, &format!("{} {}", env.tool, env.version))
}

// ---- C: compact -----------------------------------------------------------------------------

fn compact(env: &Envelope, t: &Theme) -> String {
    let mut head = Row::new()
        .add(t, "heading", &format!("{} {}", env.tool, env.version))
        .add(t, "dim", " | ");
    if let Some(s) = score_text(env) {
        head = head
            .add(t, Theme::score_token(env.score.value), &s)
            .add(t, "dim", " | ");
    }
    let mut s = head
        .add(t, "dim", &format!("{} findings: ", env.findings.len()))
        .styled;
    s += &counts_row(env, t).styled;
    s.push('\n');
    if env.findings.is_empty() {
        return s;
    }
    let w_id = env
        .findings
        .iter()
        .map(|f| f.id.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let w_loc = env
        .findings
        .iter()
        .map(|f| loc(f).chars().count())
        .max()
        .unwrap_or(8)
        .max(8);
    s += &t.paint(
        "dim",
        &format!(
            "{:<9} {:<w_id$}  {:<w_loc$}  MESSAGE",
            "SEV", "RULE", "LOCATION"
        ),
    );
    s.push('\n');
    for f in &env.findings {
        s += &Row::new()
            .add(t, f.severity.as_str(), &sev_pad(f.severity))
            .add(t, "accent", &format!(" {:<w_id$} ", f.id))
            .raw(t, &format!(" {:<w_loc$}  ", loc(f)))
            .raw(t, &clip(&f.message, 60))
            .styled;
        s.push('\n');
    }
    s
}
