//! REPL: reedline loop over `Session::exec`, with tab-completion of verbs, children, actions
//! and flags. `-c "a; b"` runs non-interactively; `--json` makes every reply one JSON record.
use crate::check::Ctx;
use crate::doctor::Doctor;
use crate::session::{complete, Hints, Outcome, Session};
use reedline::{
    default_emacs_keybindings, ColumnarMenu, Completer, DefaultPrompt, DefaultPromptSegment, Emacs,
    KeyCode, KeyModifiers, MenuBuilder, Reedline, ReedlineEvent, ReedlineMenu, Signal, Span,
    Suggestion,
};
use std::sync::{Arc, Mutex};

struct Tab(Arc<Mutex<Hints>>);

impl Completer for Tab {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let line = &line[..pos];
        let (start, words) = complete(&self.0.lock().unwrap(), line);
        words
            .into_iter()
            .map(|value| Suggestion {
                value,
                span: Span::new(start, pos),
                append_whitespace: true,
                ..Suggestion::default()
            })
            .collect()
    }
}

/// Exit code: 0, or 1 when `--json` mode swallowed a failed command.
pub fn run(doctor: &dyn Doctor, ctx: Ctx, command: Option<&str>, json: bool) -> Result<u8, String> {
    let mut s = Session::new(doctor, ctx);
    s.json = json;
    if let Some(c) = command {
        for line in c.split(';') {
            match s.exec(line)? {
                Outcome::Text(t) if !t.is_empty() => sayln!("{t}")?,
                Outcome::Quit => break,
                _ => {}
            }
        }
        return Ok(s.failed as u8);
    }
    if !json {
        sayln!(
            "{} {}: interactive. {} findings. Type `help`; Tab completes.",
            doctor.name(),
            doctor.version(),
            s.env.findings.len()
        )?;
    }
    if !s.notes.is_empty() {
        say!("{}", crate::face::notes_text(&s.notes))?;
    }
    let hints = Arc::new(Mutex::new(s.hints()));
    let mut kb = default_emacs_keybindings();
    kb.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".into()),
            ReedlineEvent::MenuNext,
        ]),
    );
    let mut ed = Reedline::create()
        .with_completer(Box::new(Tab(hints.clone())))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(
            ColumnarMenu::default().with_name("completion_menu"),
        )))
        .with_edit_mode(Box::new(Emacs::new(kb)));
    loop {
        let prompt = DefaultPrompt::new(
            DefaultPromptSegment::Basic(format!("{}:{}", doctor.name(), s.pwd())),
            DefaultPromptSegment::Empty,
        );
        match ed.read_line(&prompt).map_err(|e| e.to_string())? {
            Signal::Success(line) => {
                match s.exec(&line) {
                    Ok(Outcome::Text(t)) if !t.is_empty() => sayln!("{t}")?,
                    Ok(Outcome::Quit) => return Ok(s.failed as u8),
                    Ok(_) => {}
                    Err(e) => sayln!("error: {e}")?,
                }
                *hints.lock().unwrap() = s.hints();
            }
            Signal::CtrlC => {}
            Signal::CtrlD => return Ok(s.failed as u8),
        }
    }
}
