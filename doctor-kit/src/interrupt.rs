//! Ctrl-C (SIGINT) and SIGTERM as exit code 130, without doing anything unsafe in the handler.
//!
//! The handler only sets an atomic flag; code that can run for a while polls [`interrupted`] (or
//! [`check`]) where stopping is safe and unwinds normally. A second signal restores the default
//! disposition and re-raises, so a wedged run can still be killed with a second Ctrl-C. Handlers
//! use `SA_RESTART`, so a syscall in flight finishes before the flag is noticed.
//!
//! Needs feature `signals` (unix, `libc`). Without it, or off unix, [`install`] does nothing and
//! [`interrupted`] is always false, so callers need no `cfg`.
//!
//! The kit's `scan` installs the handler and polls between checks and before printing. A tool's own
//! command calls [`install`] itself and [`check`] in its loops. `mcp`, `shell` and `explore` never
//! install it: a server must stay killable by SIGTERM.
use crate::doctor::{failure_envelope, Doctor, ScanFailure};
use serde_json::Value;

/// Exit code of an interrupted run.
pub const EXIT: u8 = 130;

#[cfg(all(unix, feature = "signals"))]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    static INTERRUPTED: AtomicBool = AtomicBool::new(false);
    /// Saturates at 1 so a flood of signals cannot wrap back to "first".
    static INTERRUPTS: AtomicU8 = AtomicU8::new(0);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Reaction {
        Recorded,
        Reraise,
    }

    pub fn record_interrupt() -> Reaction {
        if INTERRUPTS.load(Ordering::SeqCst) == 0 {
            INTERRUPTS.store(1, Ordering::SeqCst);
            INTERRUPTED.store(true, Ordering::SeqCst);
            Reaction::Recorded
        } else {
            Reaction::Reraise
        }
    }

    extern "C" fn handle_signal(signal: libc::c_int) {
        if record_interrupt() == Reaction::Reraise {
            // SAFETY: sigaction and raise are async-signal-safe; this restores SIG_DFL for the
            // signal being handled and re-raises it, which terminates the process.
            unsafe {
                let default = disposition(libc::SIG_DFL, 0);
                libc::sigaction(signal, &default, std::ptr::null_mut());
                libc::raise(signal);
            }
        }
    }

    fn disposition(handler: libc::sighandler_t, flags: libc::c_int) -> libc::sigaction {
        // SAFETY: an all-zero `sigaction` is valid on every unix libc supports (SIG_DFL, empty
        // mask, no flags); the struct's fields differ between glibc and Apple, so no literal.
        let mut a: libc::sigaction = unsafe { std::mem::zeroed() };
        a.sa_sigaction = handler;
        a.sa_flags = flags;
        a
    }

    pub fn install() -> std::io::Result<()> {
        let a = disposition(
            handle_signal as *const () as libc::sighandler_t,
            libc::SA_RESTART,
        );
        for sig in [libc::SIGINT, libc::SIGTERM] {
            // SAFETY: `a` is fully initialised with an empty mask; null old-action is allowed.
            if unsafe { libc::sigaction(sig, &a, std::ptr::null_mut()) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    pub fn interrupted() -> bool {
        INTERRUPTED.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn reset() {
        INTERRUPTS.store(0, Ordering::SeqCst);
        INTERRUPTED.store(false, Ordering::SeqCst);
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // One test: the statics are process-global and tests run on parallel threads.
        #[test]
        fn first_signal_recorded_later_ones_reraise_and_sigterm_sets_the_flag() {
            reset();
            assert!(!interrupted());
            assert_eq!(record_interrupt(), Reaction::Recorded);
            assert!(interrupted());
            for _ in 0..3 {
                assert_eq!(record_interrupt(), Reaction::Reraise);
                assert!(interrupted(), "the flag never clears");
            }
            reset();
            install().unwrap();
            // SAFETY: raising a signal this module just installed a handler for.
            assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
            assert!(interrupted(), "SIGTERM sets the same flag");
            reset();
        }
    }
}

#[cfg(not(all(unix, feature = "signals")))]
mod imp {
    pub fn install() -> std::io::Result<()> {
        Ok(())
    }
    pub fn interrupted() -> bool {
        false
    }
}

/// Install the SIGINT/SIGTERM handlers (idempotent). A refusal from the kernel is ignored: a run
/// that cannot be interrupted cleanly still produces correct output.
pub fn install() {
    let _ = imp::install();
}

/// Whether SIGINT or SIGTERM arrived since [`install`]. One atomic load.
pub fn interrupted() -> bool {
    imp::interrupted()
}

/// The failure for an interrupted run: exit 130, `error: interrupted` on stderr and a
/// [`failure_envelope`] of kind `interrupted` (its `exit_code` is 130) for `--json`.
pub fn failure<D: Doctor + ?Sized>(d: &D) -> ScanFailure {
    let mut env = failure_envelope(d, "interrupted", "interrupted", Value::Null);
    env.exit_code = EXIT;
    ScanFailure {
        exit: EXIT,
        stderr: "error: interrupted\n".into(),
        envelope: Some(Box::new(env)),
    }
}

/// `Err(failure)` once interrupted; poll it where stopping is safe.
pub fn check<D: Doctor + ?Sized>(d: &D) -> Result<(), ScanFailure> {
    if interrupted() {
        Err(failure(d))
    } else {
        Ok(())
    }
}
