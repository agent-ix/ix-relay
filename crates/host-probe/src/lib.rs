//! Native host measurements shared by `relay-watcher` and the build scheduler.
//!
//! The crate reads the host and nothing else: no Redis, no OTLP, no shell. Linux
//! and WSL read `/proc` and `/sys`; macOS reads the Mach host interface. Every
//! reader returns an [`ErrorClass`] on failure, and [`ErrorClass::Unsupported`]
//! means the platform has no such reading (not a fault). Each parser takes text
//! or a root directory so tests drive it from fixtures; the `fixtures` feature
//! exposes those parsers on every platform.

pub mod bound;
mod error;
pub mod host;
#[cfg(target_os = "macos")]
mod mac;
pub mod pressure;
pub mod procs;

pub use error::ErrorClass;
