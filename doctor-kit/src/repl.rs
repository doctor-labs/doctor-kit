//! Minimal REPL: reedline loop over `Session::exec`. `-c "a; b"` runs non-interactively.
use crate::check::Ctx;
use crate::doctor::Doctor;
use crate::session::{Outcome, Session};
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};

pub fn run(doctor: &dyn Doctor, ctx: Ctx, command: Option<&str>) -> Result<(), String> {
    let mut s = Session::new(doctor, ctx);
    if let Some(c) = command {
        for line in c.split(';') {
            match s.exec(line)? {
                Outcome::Text(t) if !t.is_empty() => println!("{t}"),
                Outcome::Quit => break,
                _ => {}
            }
        }
        return Ok(());
    }
    println!(
        "{} {}: interactive. {} findings. Type `help`.",
        doctor.name(),
        doctor.version(),
        s.env.findings.len()
    );
    // ponytail: no tab completion yet (needs a reedline menu + Completer over VERBS and children).
    let mut ed = Reedline::create();
    loop {
        let prompt = DefaultPrompt::new(
            DefaultPromptSegment::Basic(format!("{}:{}", doctor.name(), s.pwd())),
            DefaultPromptSegment::Empty,
        );
        match ed.read_line(&prompt).map_err(|e| e.to_string())? {
            Signal::Success(line) => match s.exec(&line) {
                Ok(Outcome::Text(t)) if !t.is_empty() => println!("{t}"),
                Ok(Outcome::Quit) => return Ok(()),
                Ok(_) => {}
                Err(e) => println!("error: {e}"),
            },
            Signal::CtrlC => {}
            Signal::CtrlD => return Ok(()),
        }
    }
}
