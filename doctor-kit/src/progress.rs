//! Scan progress events (issue #29).
//!
//! A [`Progress`] sink receives [`ProgressEvent`]s from the scan loop
//! ([`crate::doctor::Doctor::collect`]). By default the scan loop uses a no-op
//! sink, so this adds no output and no dependency. A caller that wants progress
//! (e.g. a terminal progress-bar renderer) passes an `Arc<dyn Progress>` via
//! [`crate::check::Ctx::with_progress`].

use doctor_core::Severity;
use std::io::IsTerminal;
use std::io::Write as _;
use std::sync::{Arc, Mutex};

/// One step the scan loop reports. Passed by reference to [`Progress::event`]
/// so a sink can observe without taking ownership.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Just started running the check (before its `Check::run`).
    CheckStarted {
        id: String,
        index: usize,
        total: usize,
    },
    /// The check was skipped (disabled, ineligible, or its `applies` was false).
    CheckSkipped {
        id: String,
        index: usize,
        total: usize,
    },
    /// One finding that survived the category filter, in emission order.
    Finding {
        id: String,
        severity: Severity,
        message: String,
    },
    /// The check finished; `findings` is the count of `Finding` events emitted for it.
    CheckFinished { id: String, findings: usize },
    /// All checks done; `findings` is the total surviving finding count for the scan.
    Done { findings: usize },
}

/// A sink for [`ProgressEvent`]s. Implementors must be `Send + Sync` so a
/// no-op default can be shared across the scan. The `Debug` bound on the trait
/// object is avoided: `Ctx` keeps progress behind a private wrapper with a
/// manual `Debug` impl instead, so deriving `Debug`/`Clone` on `Ctx` is
/// unaffected.
pub trait Progress: Send + Sync {
    fn event(&self, e: &ProgressEvent);
}

/// No-op progress sink: absorbs every event silently. The scan loop uses this
/// when no sink was attached via [`crate::check::Ctx::with_progress`].
#[derive(Default)]
struct Noop;

impl Progress for Noop {
    #[inline]
    fn event(&self, _e: &ProgressEvent) {}
}

/// Small wrapper so `Arc<dyn Progress>` stays invisible to `Ctx`'s `Debug`
/// (a trait object's `Debug` would otherwise require `Progress: Debug`, which
/// most implementors won't satisfy and which would break `Ctx`'s derives).
#[derive(Clone)]
pub(crate) struct ProgressSink(Arc<dyn Progress>);

impl std::fmt::Debug for ProgressSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProgressSink").finish()
    }
}

impl ProgressSink {
    /// Wrap a caller-supplied sink, replacing the no-op default.
    pub(crate) fn from_arc(p: Arc<dyn Progress>) -> Self {
        Self(p)
    }
    pub(crate) fn as_progress(&self) -> &dyn Progress {
        self.0.as_ref()
    }
}

impl Default for ProgressSink {
    fn default() -> Self {
        Self(Arc::new(Noop))
    }
}

impl Progress for ProgressSink {
    #[inline]
    fn event(&self, e: &ProgressEvent) {
        self.0.event(e);
    }
}

/// Terminal status line shown while a scan runs (issue #29).
///
/// `new()` makes an empty bar (`0/0`); [`ProgressView::apply`] folds in each
/// [`ProgressEvent`]; [`ProgressView::line`] renders a single-width bar plus
/// the running state.
#[derive(Debug, Default)]
pub struct ProgressView {
    /// number of checks finished or skipped.
    done: usize,
    /// total of the latest `CheckStarted`/`CheckSkipped` (0 before any).
    total: usize,
    /// surviving finding counts per severity.
    severities: [usize; 5],
    /// message of the last `Finding`.
    latest: Option<String>,
}

impl ProgressView {
    /// An empty bar: 0 done / 0 total, no findings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one event into the view.
    pub fn apply(&mut self, e: &ProgressEvent) {
        match e {
            ProgressEvent::CheckStarted { total, .. }
            | ProgressEvent::CheckSkipped { total, .. } => {
                self.total = *total;
                if matches!(e, ProgressEvent::CheckSkipped { .. }) {
                    self.done += 1;
                }
            }
            ProgressEvent::CheckFinished { .. } => {
                self.done += 1;
            }
            ProgressEvent::Finding {
                severity, message, ..
            } => {
                self.severities[*severity as usize] += 1;
                self.latest = Some(message.clone());
            }
            ProgressEvent::Done { .. } => {
                self.done = self.total;
            }
        }
    }

    /// Render the status line truncated to `width` cells.
    pub fn line(&self, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let filled = if self.total == 0 {
            0
        } else {
            (10 * self.done).checked_div(self.total).unwrap_or(0) as u32
        };
        let mut bar: String = String::with_capacity(21);
        for _ in 0..filled {
            bar.push('#');
        }
        for _ in filled..10 {
            bar.push('-');
        }

        let mut out = format!("[{bar}] {}/{}", self.done, self.total);
        // Non-zero severities, most severe first (Severity::ALL reversed).
        for s in Severity::ALL.iter().rev() {
            let n = self.severities[*s as usize];
            if n > 0 {
                out.push_str(" \u{b7} ");
                out.push_str(&n.to_string());
                out.push(' ');
                out.push_str(&format!("{:?}", s).to_lowercase());
            }
        }
        if let Some(msg) = &self.latest {
            out.push_str(" \u{b7} latest: ");
            out.push_str(msg);
        }
        if out.chars().count() > width {
            let keep: String = out.chars().take(width - 1).collect();
            format!("{keep}\u{2026}")
        } else {
            out
        }
    }
}

/// A [`Progress`] sink that prints the status line to **stderr** when stderr is a
/// terminal, and writes nothing at all otherwise (e.g. under a pipe or in CI, so
/// machine output on stdout stays byte-identical).
pub struct StderrProgress {
    view: Mutex<ProgressView>,
}

impl Default for StderrProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl StderrProgress {
    /// Make a new stderr status-line sink.
    pub fn new() -> Self {
        Self {
            view: Mutex::new(ProgressView::new()),
        }
    }

    /// Column width to render at: the `COLUMNS` env var, else 80.
    fn cols() -> usize {
        std::env::var_os("COLUMNS")
            .and_then(|v| v.to_string_lossy().parse().ok())
            .filter(|&c: &usize| c > 0)
            .unwrap_or(80)
    }
}

impl Progress for StderrProgress {
    fn event(&self, e: &ProgressEvent) {
        if !std::io::stderr().is_terminal() {
            return;
        }
        let mut view = self.view.lock().unwrap();
        view.apply(e);
        let line = view.line(Self::cols());
        if matches!(e, ProgressEvent::Done { .. }) {
            // Clear the line on completion.
            let mut err = std::io::stderr();
            let _ = err.write_all(b"\r\x1b[K");
        } else {
            let mut err = std::io::stderr();
            let _ = err.write_fmt(format_args!("\r{line}\x1b[K"));
        }
    }
}
