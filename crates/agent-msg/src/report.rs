//! Stderr that cannot panic.
//!
//! `eprintln!` panics when stderr is closed (a harness that drops the pipe
//! gives `EPIPE`), and a panic is exactly what a hook must never turn a
//! diagnostic into. Everything a hook prints on stderr goes through here.

use std::{fmt, io::Write};

/// Writes `args` and a newline to stderr, ignoring any failure to do so.
pub(crate) fn line(args: fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr(), "{args}");
}
