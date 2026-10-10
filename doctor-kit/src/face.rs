//! Faces: renderers over a doctor/1 `Envelope`. Faces read theme tokens only.
use crate::doctor::{is_scored, severity_counts};
use crate::theme::Theme;
use doctor_core::{Envelope, Finding, Severity};
use serde_json::Value;

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

/// A copy of `env` safe to show a human (contract section 8): control, escape, bidi and
/// zero-width characters are removed from every finding text that can come from scanned input
/// (id, category, message, location, evidence, remedy). Findings, fingerprints and `data` are
/// otherwise untouched; machine output (`--json`, SARIF) is never passed through this.
pub fn sanitize_envelope(env: &Envelope) -> Envelope {
    use doctor_core::sanitize;
    let mut e = env.clone();
    for f in &mut e.findings {
        f.id = sanitize(&f.id);
        f.category = sanitize(&f.category);
        f.message = sanitize(&f.message);
        f.location.reference = sanitize(&f.location.reference);
        for ev in &mut f.evidence {
            ev.reference = sanitize(&ev.reference);
            ev.value = sanitize(&ev.value);
        }
        f.remedy = f.remedy.as_deref().map(sanitize);
    }
    e
}

/// How loud a [`Note`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteLevel {
    Info,
    Warn,
    Error,
}

impl NoteLevel {
    fn parse(s: &str) -> NoteLevel {
        match s.to_ascii_lowercase().as_str() {
            "info" | "note" => NoteLevel::Info,
            "error" | "critical" | "fatal" => NoteLevel::Error,
            _ => NoteLevel::Warn,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            NoteLevel::Info => "info",
            NoteLevel::Warn => "warn",
            NoteLevel::Error => "error",
        }
    }
    /// Label shown before the text.
    fn label(self) -> &'static str {
        match self {
            NoteLevel::Info => "NOTE: ",
            NoteLevel::Warn => "WARNING: ",
            NoteLevel::Error => "ERROR: ",
        }
    }
    /// Theme token used to paint it.
    fn token(self) -> &'static str {
        match self {
            NoteLevel::Info => "dim",
            NoteLevel::Warn => "medium",
            NoteLevel::Error => "high",
        }
    }
}

/// A line the human report shows before everything else (the result is incomplete, the walk was
/// truncated, an archive stopped early). Never part of the machine envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub level: NoteLevel,
    pub text: String,
}

/// The default notes of an envelope: an `INCOMPLETE` banner when `score.coverage_gaps > 0`, then
/// `data.notes` (an array of strings, taken as warnings, or of `{"level", "text"}` objects).
pub fn notes_of(env: &Envelope) -> Vec<Note> {
    let mut v = vec![];
    if env.score.coverage_gaps > 0 {
        v.push(Note {
            level: NoteLevel::Warn,
            text: format!(
                "INCOMPLETE: {} coverage gap(s); findings may be missing",
                env.score.coverage_gaps
            ),
        });
    }
    for n in env.data["notes"].as_array().into_iter().flatten() {
        match n {
            Value::String(t) => v.push(Note {
                level: NoteLevel::Warn,
                text: t.clone(),
            }),
            Value::Object(o) => {
                if let Some(t) = o.get("text").and_then(Value::as_str) {
                    let l = o.get("level").and_then(Value::as_str).unwrap_or("warn");
                    v.push(Note {
                        level: NoteLevel::parse(l),
                        text: t.to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    v
}

/// `lines` of notes for plain text (no theme), sanitized.
pub fn notes_text(notes: &[Note]) -> String {
    notes
        .iter()
        .map(|n| format!("{}{}\n", n.level.label(), doctor_core::sanitize(&n.text)))
        .collect()
}

pub fn render(face: Face, env: &Envelope, theme: &Theme) -> String {
    render_with_notes(face, env, theme, &notes_of(env))
}

/// [`render`] with the notes chosen by the caller (see [`crate::Doctor::notes`]). Notes are shown
/// first, in the face's own style; their text is sanitized like finding text.
pub fn render_with_notes(face: Face, env: &Envelope, theme: &Theme, notes: &[Note]) -> String {
    let env = &sanitize_envelope(env);
    match face {
        Face::Plain => plain(env, theme, notes),
        Face::Rich => rich(env, theme, notes),
        Face::Compact => compact(env, theme, notes),
    }
}

fn note_row(n: &Note, t: &Theme, max: usize) -> Row {
    let text = doctor_core::sanitize(&n.text);
    Row::new().add(
        t,
        n.level.token(),
        &clip(&format!("{}{text}", n.level.label()), max),
    )
}

fn notes_block(t: &Theme, notes: &[Note]) -> String {
    notes
        .iter()
        .map(|n| note_row(n, t, usize::MAX).styled + "\n")
        .collect()
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

fn plain(env: &Envelope, t: &Theme, notes: &[Note]) -> String {
    let mut out = Row::new().add(t, "heading", &format!("{} {}", env.tool, env.version));
    if let Some(s) = score_text(env) {
        out = out
            .add(t, "dim", "  score ")
            .add(t, Theme::score_token(env.score.value), &s);
    }
    let mut s = notes_block(t, notes) + &out.styled + "\n";
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

fn rich(env: &Envelope, t: &Theme, notes: &[Note]) -> String {
    let mut rows = vec![];
    for n in notes {
        rows.push(Some(note_row(n, t, INNER)));
    }
    if !notes.is_empty() {
        rows.push(None);
    }
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

fn compact(env: &Envelope, t: &Theme, notes: &[Note]) -> String {
    let mut head = Row::new()
        .add(t, "heading", &format!("{} {}", env.tool, env.version))
        .add(t, "dim", " | ");
    if let Some(s) = score_text(env) {
        head = head
            .add(t, Theme::score_token(env.score.value), &s)
            .add(t, "dim", " | ");
    }
    let mut s = notes_block(t, notes)
        + &head
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
