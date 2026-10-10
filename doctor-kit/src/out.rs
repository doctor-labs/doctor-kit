//! Writing to stdout without panicking. `print!` panics when stdout is closed
//! (`tool scan | head`), which would exit 101; the contract says exit 2 (the tool could not run).
use std::io::Write;

/// Write all of `text` to stdout and flush. A closed pipe or full disk is an `Err`, never a panic.
pub fn say(text: &str) -> Result<(), String> {
    let mut o = std::io::stdout().lock();
    o.write_all(text.as_bytes())
        .and_then(|_| o.flush())
        .map_err(|e| format!("cannot write to stdout: {e}"))
}
