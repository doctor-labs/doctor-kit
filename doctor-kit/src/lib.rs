//! doctor-kit: the reusable runtime for doctor CLI tools.
//!
//! A tool implements [`Doctor`] (name, version, categories, checks) and calls [`run`].
//! Output is a `doctor/1` [`Envelope`] rendered by a [`Face`] in a [`Theme`].
/// stderr that never panics (a closed stderr loses the message, nothing more).
macro_rules! warnln {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}
macro_rules! warn {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = write!(std::io::stderr(), $($t)*);
    }};
}
/// stdout that reports a failure instead of panicking: `sayln!(..)?` in a function returning
/// `Result<_, String>` (a closed pipe, as in `tool scan | head`, becomes an error, exit 2).
macro_rules! say {
    ($($t:tt)*) => {
        $crate::out::say(&format!($($t)*))
    };
}
macro_rules! sayln {
    ($($t:tt)*) => {
        $crate::out::say(&format!("{}\n", format_args!($($t)*)))
    };
}

pub mod baseline;
pub mod check;
pub mod cli;
pub mod config;
pub mod doctor;
pub mod face;
pub mod fix;
pub mod install;
pub mod interrupt;
pub mod node;
pub mod out;
pub mod output;
pub mod progress;
pub mod session;
pub mod theme;

pub mod mcp;
#[cfg(feature = "repl")]
pub mod repl;
#[cfg(feature = "tui")]
pub mod tui;

pub use check::{finding, Check, CheckResult, Ctx, Scope};
pub use cli::{command_with, run, run_with, ExtCommand, Extensions, Meta};
pub use config::{Config, Weights};
pub use doctor::{failure_envelope, Collected, Doctor, PreflightKind, ScanFailure, TargetKind};
pub use face::{Face, Note, NoteLevel};
pub use mcp::{McpTexts, McpTool, McpWire, Protocol};
pub use node::{Action, Node};
pub use output::{finish, finish_with, preflight, FinishOpts, OutputArgs};
pub use progress::{Progress, ProgressEvent, ProgressView, StderrProgress};
pub use session::{Command, Reply, Session};
pub use theme::Theme;

pub use doctor_core;
