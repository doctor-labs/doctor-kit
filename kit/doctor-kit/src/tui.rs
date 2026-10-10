//! TUI: the 4-pane layout of all-in-one-cli.md section 4 over the same `Session` commands.
//! Keys: j/k or arrows move, enter/l descends, backspace/h ascends, `/` filter, `:` command
//! line, `d` decode, `w` why, `x` export, tab switches tree/findings focus, `q` quits.
use crate::check::Ctx;
use crate::doctor::Doctor;
use crate::session::{Outcome, Session};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

#[derive(PartialEq, Eq)]
enum Mode {
    Normal,
    Filter,
    Command,
}

struct App<'a> {
    s: Session<'a>,
    sel: usize,
    fsel: usize,
    filter: String,
    input: String,
    mode: Mode,
    findings_focus: bool,
    detail: String,
    note: String,
}

impl<'a> App<'a> {
    fn new(s: Session<'a>) -> App<'a> {
        let mut a = App {
            s,
            sel: 0,
            fsel: 0,
            filter: String::new(),
            input: String::new(),
            mode: Mode::Normal,
            findings_focus: false,
            detail: String::new(),
            note: String::new(),
        };
        a.refresh();
        a
    }

    fn names(&self) -> Vec<String> {
        self.s
            .entries(&self.filter)
            .iter()
            .map(|n| n.name.clone())
            .collect()
    }

    /// Global finding indexes passing the filter (the bottom pane).
    fn findings(&self) -> Vec<usize> {
        let q = self.filter.to_lowercase();
        (0..self.s.env.findings.len())
            .filter(|&i| {
                let f = &self.s.env.findings[i];
                q.is_empty()
                    || [&f.id, &f.message, &f.location.reference]
                        .iter()
                        .any(|t| t.to_lowercase().contains(&q))
            })
            .collect()
    }

    fn clamp(&mut self) {
        self.sel = self.sel.min(self.names().len().saturating_sub(1));
        self.fsel = self.fsel.min(self.findings().len().saturating_sub(1));
    }

    /// Run a command through the shared session; its text lands in the detail pane.
    fn run(&mut self, line: &str) -> Option<bool> {
        match self.s.exec(line) {
            Ok(Outcome::Text(t)) => {
                self.note = t.lines().next().unwrap_or("").to_string();
                self.detail = t;
                Some(true)
            }
            Ok(Outcome::Quit) => None,
            Err(e) => {
                self.note = e.clone();
                self.detail = e;
                Some(false)
            }
        }
    }

    fn refresh(&mut self) {
        self.clamp();
        let line = self
            .names()
            .get(self.sel)
            .map_or("info".to_string(), |n| format!("cat {n}"));
        self.run(&line);
        self.note.clear();
    }

    /// Finding to explain: the focused list row, else the selected node's.
    fn finding_here(&self) -> Option<usize> {
        if self.findings_focus {
            return self.findings().get(self.fsel).copied();
        }
        let names = self.names();
        let n = names
            .get(self.sel)
            .and_then(|n| self.s.node(Some(n)).ok())?;
        n.finding
    }

    /// false = quit.
    fn key(&mut self, c: KeyCode) -> bool {
        match self.mode {
            Mode::Normal => return self.normal(c),
            Mode::Filter => match c {
                KeyCode::Enter => self.mode = Mode::Normal,
                KeyCode::Esc => {
                    self.filter.clear();
                    self.mode = Mode::Normal;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(ch) => self.filter.push(ch),
                _ => {}
            },
            Mode::Command => match c {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(ch) => self.input.push(ch),
                KeyCode::Enter => {
                    let line = std::mem::take(&mut self.input);
                    self.mode = Mode::Normal;
                    if self.run(&line).is_none() {
                        return false;
                    }
                    self.sel = 0;
                    self.clamp();
                    return true;
                }
                _ => {}
            },
        }
        self.sel = 0;
        self.fsel = 0;
        self.clamp();
        if self.mode == Mode::Normal {
            self.refresh();
        }
        true
    }

    fn normal(&mut self, c: KeyCode) -> bool {
        let n = if self.findings_focus {
            self.findings().len()
        } else {
            self.names().len()
        };
        let cur = if self.findings_focus {
            &mut self.fsel
        } else {
            &mut self.sel
        };
        match c {
            KeyCode::Char('q') => return false,
            KeyCode::Down | KeyCode::Char('j') => *cur = (*cur + 1).min(n.saturating_sub(1)),
            KeyCode::Up | KeyCode::Char('k') => *cur = cur.saturating_sub(1),
            KeyCode::Tab => self.findings_focus = !self.findings_focus,
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') if !self.findings_focus => {
                if let Some(name) = self.names().get(self.sel).cloned() {
                    self.run(&format!("cd {name}"));
                    self.sel = 0;
                    self.filter.clear();
                }
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                self.run("cd ..");
                self.sel = 0;
                self.filter.clear();
            }
            KeyCode::Char('/') => {
                self.filter.clear();
                self.mode = Mode::Filter;
            }
            KeyCode::Char(':') => {
                self.input.clear();
                self.mode = Mode::Command;
            }
            KeyCode::Char('d') => {
                let line = self
                    .names()
                    .get(self.sel)
                    .map_or("decode".to_string(), |n| format!("decode {n}"));
                self.refresh();
                self.run(&line);
                return true;
            }
            KeyCode::Char('w') => {
                self.refresh();
                match self.finding_here() {
                    Some(i) => {
                        self.run(&format!("why {}", i + 1));
                    }
                    None => self.note = "no finding at the selection".into(),
                }
                return true;
            }
            KeyCode::Char('x') => {
                let path = format!("{}-export.json", self.s.tool());
                self.refresh();
                self.run(&format!("export json {path}"));
                return true;
            }
            _ => {}
        }
        self.refresh();
        true
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
    let hl = Style::new().add_modifier(Modifier::REVERSED);
    let focus = |t: &str, on: bool| format!(" {t}{} ", if on { " *" } else { "" });

    let entries = app.s.entries(&app.filter);
    let items: Vec<ListItem> = entries
        .iter()
        .map(|n| ListItem::new(format!("{} {}", n.name, n.summary)))
        .collect();
    let mut st = ListState::default().with_selected((!entries.is_empty()).then_some(app.sel));
    let title = if app.filter.is_empty() {
        app.s.pwd()
    } else {
        format!("{} /{}", app.s.pwd(), app.filter)
    };
    f.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().title(focus(&title, !app.findings_focus)))
            .highlight_style(hl),
        left,
        &mut st,
    );

    f.render_widget(
        Paragraph::new(app.detail.clone())
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(" detail ")),
        detail,
    );
    let node = entries
        .get(app.sel)
        .copied()
        .unwrap_or_else(|| app.s.here());
    let raw_text = match &node.raw {
        Some(b) => crate::node::hexdump(b),
        None => serde_json::to_string_pretty(&node.meta).unwrap_or_default(),
    };
    f.render_widget(
        Paragraph::new(raw_text).block(Block::bordered().title(" hex / raw ")),
        raw,
    );

    let idx = app.findings();
    let rows: Vec<ListItem> = idx
        .iter()
        .map(|&i| {
            let fd = &app.s.env.findings[i];
            ListItem::new(format!(
                "{:>2} {:<9} {} {}",
                i + 1,
                fd.severity.as_str(),
                fd.id,
                fd.location.reference
            ))
        })
        .collect();
    let mut fs = ListState::default().with_selected((!idx.is_empty()).then_some(app.fsel));
    f.render_stateful_widget(
        List::new(rows)
            .block(Block::bordered().title(focus("findings", app.findings_focus)))
            .highlight_style(hl),
        findings,
        &mut fs,
    );

    let line = match app.mode {
        Mode::Filter => format!("/{}", app.filter),
        Mode::Command => format!(":{}", app.input),
        Mode::Normal => format!(
            " {} | j/k move  enter cd  bksp up  / filter  : cmd  d decode  w why  x export  tab focus  q quit | {}",
            app.s.ctx.root.display(),
            app.note
        ),
    };
    f.render_widget(Paragraph::new(line), status);
}

pub fn run(doctor: &dyn Doctor, ctx: Ctx) -> Result<(), String> {
    let mut app = App::new(Session::new(doctor, ctx));
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
    use crate::node::Node;
    use crate::{finding, Check, CheckResult};
    use doctor_core::Severity;
    use ratatui::{backend::TestBackend, Terminal};

    struct One;
    impl Check for One {
        fn id(&self) -> &'static str {
            "t.one"
        }
        fn category(&self) -> &'static str {
            "c"
        }
        fn run(&self, _: &Ctx) -> CheckResult {
            CheckResult::findings(vec![
                finding(
                    "t.one",
                    "c",
                    Severity::High,
                    "bad thing",
                    std::path::Path::new("a"),
                    Some(1),
                ),
                finding(
                    "t.two",
                    "c",
                    Severity::Low,
                    "meh",
                    std::path::Path::new("b"),
                    None,
                ),
            ])
        }
    }
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
        fn checks(&self) -> Vec<Box<dyn Check>> {
            vec![Box::new(One)]
        }
        fn tree(&self, env: &doctor_core::Envelope, ctx: &Ctx) -> Node {
            let mut t = crate::session::default_tree(env, ctx);
            t.children
                .push(Node::new("blob", "bytes", "raw thing").raw(b"Hello, hex pane".to_vec()));
            t
        }
    }

    fn screen(app: &App) -> String {
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| draw(f, app)).unwrap();
        format!("{}", t.backend())
    }

    fn app() -> App<'static> {
        let dir = std::env::temp_dir().join("doctor-kit-tui-test");
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = Ctx::new(&dir, crate::Scope::Full, Default::default(), Severity::High).unwrap();
        App::new(Session::new(&D, ctx))
    }

    fn keys(a: &mut App, s: &str) {
        for c in s.chars() {
            a.key(match c {
                '\n' => KeyCode::Enter,
                '\t' => KeyCode::Tab,
                '\u{8}' => KeyCode::Backspace,
                '\u{1b}' => KeyCode::Esc,
                c => KeyCode::Char(c),
            });
        }
    }

    #[test]
    fn four_panes_render_and_share_the_session() {
        let mut a = app();
        let out = screen(&a);
        for pane in ["detail", "hex / raw", "findings", "blob"] {
            assert!(out.contains(pane), "missing {pane}");
        }
        keys(&mut a, "jj"); // category c, blob... sel on blob
        assert!(
            screen(&a).contains("48 65 6C 6C 6F"),
            "hex pane shows raw bytes"
        );
        keys(&mut a, "d");
        assert!(
            a.detail.contains("0000"),
            "decode result in detail: {}",
            a.detail
        );
    }

    #[test]
    fn filter_command_why_and_export() {
        let mut a = app();
        keys(&mut a, "/blob\n");
        assert_eq!(a.names(), ["blob"]);
        keys(&mut a, "/\u{1b}"); // esc clears
        assert_eq!(a.names().len(), 2);
        keys(&mut a, "\n"); // cd c
        assert_eq!(a.s.pwd(), "/c");
        keys(&mut a, "w");
        assert!(
            a.detail.contains("t.one"),
            "why of selected finding: {}",
            a.detail
        );
        keys(&mut a, ":pwd\n");
        assert_eq!(a.detail, "/c");
        keys(&mut a, "\u{8}"); // backspace up
        assert_eq!(a.s.pwd(), "/");
        keys(&mut a, "\t");
        assert!(a.findings_focus);
        keys(&mut a, "jw");
        assert!(a.detail.contains("t.two"), "{}", a.detail);
        assert!(!a.key(KeyCode::Char('q')));
    }
}
