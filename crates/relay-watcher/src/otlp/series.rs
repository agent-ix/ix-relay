//! The catalogue of what the watcher emits: every instrument, once, with its
//! name, unit, kind and description. FR-045 documents the same list with its
//! attributes, and a test pins both (`tc_045_11_...`).
//!
//! Adding a metric is a new `Instrument` here, a line in `CATALOGUE`, a row in
//! the spec, and the few lines that fill it.

use super::registry::{Instrument, Kind};

/// `herdr.pane.info`: one series per live pane of this machine, value 1, whose
/// attributes are the names a dashboard joins to the ids other tools emit.
pub static PANE_INFO: Instrument = Instrument {
    name: "herdr.pane.info",
    unit: "1",
    description: "Names for the herdr panes on this machine (value 1); join on machine, pane.",
    kind: Kind::Gauge,
};

/// `herdr.pane.status`: the status herdr reports for each pane.
pub static PANE_STATUS: Instrument = Instrument {
    name: "herdr.pane.status",
    unit: "1",
    description: "The status herdr reports for each pane (value 1).",
    kind: Kind::Gauge,
};

/// `ix.relay.messages`: messages by what became of them, since the watcher started.
pub static MESSAGES: Instrument = Instrument {
    name: "ix.relay.messages",
    unit: "{message}",
    description: "Messages by outcome since this watcher started: delivered counts messages of batches this watcher delivered and saw confirmed, by typing into a pane or by a socket post confirmed within its wait (not hook deliveries, and not a held message approved later); requeued, redelivered and dead_lettered are what it put back or reaped; socket denials, postponements and deferrals, sent and denied are not counted.",
    kind: Kind::Sum,
};

/// `ix.relay.inbox.depth`: messages waiting in a lead's inbox.
pub static INBOX_DEPTH: Instrument = Instrument {
    name: "ix.relay.inbox.depth",
    unit: "{message}",
    description: "Messages waiting in a local lead's inbox.",
    kind: Kind::Gauge,
};

/// `ix.relay.inbox.oldest_age`: how long a lead's oldest waiting message has waited.
pub static INBOX_OLDEST_AGE: Instrument = Instrument {
    name: "ix.relay.inbox.oldest_age",
    unit: "s",
    description: "Seconds the oldest message in a local lead's inbox has waited (0 when empty).",
    kind: Kind::Gauge,
};

/// `ix.relay.inflight.entries`: entries popped and not yet acknowledged.
pub static INFLIGHT_ENTRIES: Instrument = Instrument {
    name: "ix.relay.inflight.entries",
    unit: "{message}",
    description: "Messages a local lead has had popped and not yet acknowledged (in-flight entries, not batches).",
    kind: Kind::Gauge,
};

/// `ix.relay.deadletter.entries`: messages parked as dead letters.
pub static DEADLETTER_ENTRIES: Instrument = Instrument {
    name: "ix.relay.deadletter.entries",
    unit: "{message}",
    description: "Dead-lettered messages held for a local lead.",
    kind: Kind::Gauge,
};

/// `ix.relay.hook.failures.recent`: recent hook failures this machine reported.
pub static HOOK_FAILURES: Instrument = Instrument {
    name: "ix.relay.hook.failures.recent",
    unit: "{failure}",
    description: "Hook failures of this machine still in the capped health:hooks stream that \
                  happened since the watcher started, by stage and error kind. A gauge: it can \
                  fall when the stream drops old entries; it is not a cumulative counter.",
    kind: Kind::Gauge,
};

/// Bucket upper bounds of the tick histogram, in seconds.
pub const TICK_BOUNDS: &[f64] = &[0.005, 0.025, 0.1, 0.25, 1.0, 2.5, 10.0];

/// `ix.relay.watcher.tick.duration`: how long the watcher's ticks take.
pub static TICK_DURATION: Instrument = Instrument {
    name: "ix.relay.watcher.tick.duration",
    unit: "s",
    description: "How long one watcher tick took, since the watcher started.",
    kind: Kind::Histogram {
        bounds: TICK_BOUNDS,
    },
};

/// `ix.relay.registrations`: registered leads on this machine.
pub static REGISTRATIONS: Instrument = Instrument {
    name: "ix.relay.registrations",
    unit: "{registration}",
    description: "Leads registered on this machine.",
    kind: Kind::Gauge,
};

/// Every instrument the watcher emits, with the attribute keys each carries.
/// The spec (FR-045) lists the same, and tests compare the two and what a full
/// push actually exports.
#[cfg(test)]
pub(crate) const CATALOGUE: &[(&Instrument, &[&str])] = &[
    (
        &PANE_INFO,
        &[
            "machine",
            "pane",
            "tab",
            "workspace",
            "workspace_label",
            "tab_label",
            "session",
            "bus_name",
        ],
    ),
    (&PANE_STATUS, &["machine", "pane", "status"]),
    (&MESSAGES, &["outcome"]),
    (&INBOX_DEPTH, &["lead"]),
    (&INBOX_OLDEST_AGE, &["lead"]),
    (&INFLIGHT_ENTRIES, &["lead"]),
    (&DEADLETTER_ENTRIES, &["lead"]),
    (&HOOK_FAILURES, &["stage", "error_kind"]),
    (&TICK_DURATION, &[]),
    (&REGISTRATIONS, &["machine"]),
];
