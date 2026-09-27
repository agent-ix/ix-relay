//! Escalation orchestration shared by `agent-msg` (the CLI) and `relay-tui`
//! (the operator's inbox): investigate (fork or fresh through herdr), resolve
//! (validate, notify the lead, post to Linear), and retry-post-linear.
//!
//! Split out of `agent-msg` (AGE-2015) so `relay-tui` can call exactly the
//! same orchestration instead of reimplementing it — the escalation rules
//! (validate before notifying, close a dangling tab on a failed investigate,
//! a Linear failure is a warning not a command failure, ...) live in one
//! place. `agent-msg`-only concerns (`send`/`inbox`/`hook`/`who`/`log`, the
//! hook handler, clap wiring) stay in `agent-msg` itself.

pub mod errors;
pub mod escalate;
pub mod find;
pub mod herdr;
pub mod herdr_check;
pub mod lenient;
pub mod linear;
pub mod naming;

pub use errors::{OpsError, Result};
pub use escalate::{RESOLUTION_COMMENT_MARKER_PREFIX, resolution_comment_marker};

/// Local wall-clock time for display, e.g. `09-24 19:40`. Shared by
/// `agent-msg`'s `escalations`/`log` output and `relay-tui`'s detail pane.
#[must_use]
pub fn clock(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| "?".to_owned())
}
