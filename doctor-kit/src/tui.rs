//! Minimal TUI: the 4-pane layout of all-in-one-cli.md section 4 over `Session`.
//! Keys: j/k or arrows move, enter descends, backspace ascends, w why, x export, q quit.
use crate::check::Ctx;
use crate::doctor::Doctor;
use crate::session::{Cursor, Outcome, Session};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

struct App<'a> {
    s: Session<'a>,
    sel: usize,
    detail: Option<String>,
    note: String,
}

impl App<'_> {
    /// Entries at the cursor: category names at Root, finding indexes below.
    fn entries(&self) -> Vec<(String, Option<usize>)> {
        match self.s.cursor {
            Cursor::Root => self.s.categories().into_iter().map(|c| (c, None)).collect(),
            _ => self
                .s
                .visible()
                .into_iter()
                .map(|i| {
                    (
                        format!(
                            "{} {}",
                            self.s.env.findings[i].severity, self.s.env.findings[i].id
                        ),
                        Some(i),
                    )
                })
                .collect(),
        }
    }

    fn selected_finding(&self) -> Option<usize> {
        match self.s.cursor {
            Cursor::Finding(i) => Some(i),
            _ => self.entries().get(self.sel).and_then(|e| e.1),
        }
    }

    fn key(&mut self, c: KeyCode) -> bool {
        let n = self.entries().len();
        self.detail = None;
        match c {
            KeyCode::Char('q') => return false,
            KeyCode::Down | KeyCode::Char('j') => {
                self.sel = (self.sel + 1).min(n.saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Enter => {
                if let Some((name, idx)) = self.entries().get(self.sel).cloned() {
                    let arg = idx.map_or(name, |i| (i + 1).to_string());
                    self.run(&format!("cd {arg}"));
                    self.sel = 0;
                }
            }
            KeyCode::Backspace => {
                self.run("cd ..");
                self.sel = 0;
            }
            KeyCode::Char('w') => {
                if let Some(i) = self.selected_finding() {
                    self.detail = self.run(&format!("why {}", i + 1));
                }
            }
            KeyCode::Char('x') => {
                self.run("export json doctor-export.json");
            }
            _ => {}
        }
        true
    }

    fn run(&mut self, line: &str) -> Option<String> {
        match self.s.exec(line) {
            Ok(Outcome::Text(t)) => {
                self.note = t.clone();
                Some(t)
            }
            Ok(Outcome::Quit) => None,
            Err(e) => {
                self.note = e;
                None
            }
        }
    }
}

fn draw(f: &mut Frame, app: &App) {
    let [top, findings, status] = Layout::vertical([
        Constraint::Min(8),
        Constraint::Length(8),
        Constraint::Length(1),
    ])
    .areas(f.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(top);
    let [detail, raw] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(right);

    let items: Vec<ListItem> = app
        .entries()
        .into_iter()
        .map(|e| ListItem::new(e.0))
        .collect();
    let mut st = ListState::default().with_selected(Some(app.sel));
    f.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().title(format!(" {} ", app.s.pwd())))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        left,
        &mut st,
    );

    let sel = app.selected_finding();
    let detail_text = app.detail.clone().unwrap_or_else(|| match sel {
        Some(i) => {
            let fd = &app.s.env.findings[i];
            format!(
                "{} [{}]\n{}\n{}",
                fd.id, fd.severity, fd.location.reference, fd.message
            )
        }
        None => {
            app.s.env.tool.clone()
                + " "
                + &app.s.env.version
                + &format!("\nscore {}", app.s.env.score.value)
        }
    });
    f.render_widget(
        Paragraph::new(detail_text)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" detail ")),
        detail,
    );
    let raw_text = sel
        .map(|i| serde_json::to_string_pretty(&app.s.env.findings[i]).unwrap_or_default())
        .unwrap_or_default();
    f.render_widget(
        Paragraph::new(raw_text).block(Block::bordered().title(" raw ")),
        raw,
    );

    let rows: Vec<ListItem> = app
        .s
        .env
        .findings
        .iter()
        .enumerate()
        .map(|(i, fd)| {
            ListItem::new(format!(
                "{:>2} {:<9} {} {}",
                i + 1,
                fd.severity.as_str(),
                fd.id,
                fd.location.reference
            ))
        })
        .collect();
    f.render_widget(
        List::new(rows).block(Block::bordered().title(" findings ")),
        findings,
    );
    f.render_widget(
        Paragraph::new(format!(
            " {} | j/k move  enter cd  bksp up  w why  x export  q quit | {}",
            app.s.ctx.root.display(),
            app.note.lines().next().unwrap_or("")
        )),
        status,
    );
}

pub fn run(doctor: &dyn Doctor, ctx: Ctx) -> Result<(), String> {
    let mut app = App {
        s: Session::new(doctor, ctx),
        sel: 0,
        detail: None,
        note: String::new(),
    };
    let mut term = ratatui::init();
    let res = (|| -> std::io::Result<()> {
        loop {
            term.draw(|f| draw(f, &app))?;
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press && !app.key(k.code) {
                    return Ok(());
                }
            }
        }
    })();
    ratatui::restore();
    res.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn four_panes_render() {
        struct D;
        impl Doctor for D {
            fn name(&self) -> &str {
                "t"
            }
            fn version(&self) -> &str {
                "0"
            }
            fn categories(&self) -> Vec<&'static str> {
                vec!["c"]
            }
            fn checks(&self) -> Vec<Box<dyn crate::Check>> {
                vec![]
            }
        }
        let dir = std::env::temp_dir().join("doctor-kit-tui-test");
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = Ctx::new(
            &dir,
            crate::Scope::Full,
            Default::default(),
            doctor_core::Severity::High,
        );
        let ctx = ctx.unwrap();
        let app = App {
            s: Session::new(&D, ctx),
            sel: 0,
            detail: None,
            note: String::new(),
        };
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| draw(f, &app)).unwrap();
        let out = format!("{}", t.backend());
        for pane in ["detail", "raw", "findings"] {
            assert!(out.contains(pane), "missing {pane}");
        }
    }
}
