//! Scan progress events (issue #29).
//!
//! A [`Progress`] sink receives [`ProgressEvent`]s from the scan loop
//! ([`crate::doctor::Doctor::collect`]). By default the scan loop uses a no-op
//! sink, so this adds no output and no dependency. A caller that wants progress
//! (e.g. a terminal progress-bar renderer) passes an `Arc<dyn Progress>` via
//! [`crate::check::Ctx::with_progress`].

use doctor_core::Severity;
use std::sync::Arc;

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
