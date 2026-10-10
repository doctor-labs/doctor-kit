//! doctor-kit: the reusable runtime for doctor CLI tools.
//!
//! A tool implements [`Doctor`] (name, version, categories, checks) and calls [`run`].
//! Output is a `doctor/1` [`Envelope`] rendered by a [`Face`] in a [`Theme`].
pub mod baseline;
pub mod check;
pub mod cli;
pub mod config;
pub mod doctor;
pub mod face;
pub mod fix;
pub mod install;
pub mod node;
pub mod output;
pub mod session;
pub mod theme;

#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "repl")]
pub mod repl;
#[cfg(feature = "tui")]
pub mod tui;

pub use check::{finding, Check, CheckResult, Ctx, Scope};
pub use cli::run;
pub use config::{Config, Weights};
pub use doctor::{failure_envelope, Collected, Doctor, ScanFailure, TargetKind};
pub use face::Face;
pub use node::{Action, Node};
pub use output::{finish, preflight, OutputArgs};
pub use session::{Command, Reply, Session};
pub use theme::Theme;

pub use doctor_core;
