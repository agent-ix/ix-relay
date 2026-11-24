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
                  happened since the watcher started, by stage and error kind, each record id counted once. A gauge: it can \
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

/// Failed reads used to build the watcher's relay-health snapshot, by bounded source.
pub static HEALTH_SNAPSHOT_READ_FAILURES: Instrument = Instrument {
    name: "ix.relay.watcher.health_snapshot.read_failures",
    unit: "{failure}",
    description: "Failed reads used to build the relay-health snapshot since this watcher started, by snapshot source.",
    kind: Kind::Sum,
};

macro_rules! fleet_gauge {
    ($(#[$doc:meta])* $id:ident, $name:literal, $unit:literal, $desc:literal) => {
        $(#[$doc])*
        pub static $id: Instrument = Instrument {
            name: $name,
            unit: $unit,
            description: $desc,
            kind: Kind::Gauge,
        };
    };
}

fleet_gauge!(
    /// `ix.fleet.lane.state`: one series per registry lane.
    FLEET_LANE_STATE, "ix.fleet.lane.state", "1",
    "One series per lane of the lane registry, value 1, by state (working, idle, blocked)."
);
fleet_gauge!(
    /// `ix.fleet.lane.last_activity`: seconds since the lane's newest activity.
    FLEET_LANE_LAST_ACTIVITY, "ix.fleet.lane.last_activity", "s",
    "Seconds since the lane's newest activity event, or since the watcher first saw the lane."
);
fleet_gauge!(
    /// `ix.fleet.lane.blocked`: a blocked lane and why.
    FLEET_LANE_BLOCKED, "ix.fleet.lane.blocked", "1",
    "Value 1 for a blocked lane, by reason."
);
fleet_gauge!(
    /// `ix.fleet.lane.dropped`: registry lanes left out past the cap.
    FLEET_LANE_DROPPED, "ix.fleet.lane.dropped", "{lane}",
    "Registry lanes left out of the per-lane series because more than 50 exist."
);
fleet_gauge!(
    /// `ix.fleet.host.busy_ratio`: measured CPU busy ratio.
    FLEET_BUSY_RATIO, "ix.fleet.host.busy_ratio", "1",
    "Measured CPU busy ratio, 0 to 1; absent when its source fails."
);
fleet_gauge!(
    /// `ix.fleet.host.memory_used_bytes`.
    FLEET_MEMORY_USED, "ix.fleet.host.memory_used_bytes", "By",
    "Memory in use on this host (total minus available); absent when its source fails."
);
fleet_gauge!(
    /// `ix.fleet.host.memory_total_bytes`.
    FLEET_MEMORY_TOTAL, "ix.fleet.host.memory_total_bytes", "By",
    "Total memory of this host; absent when its source fails."
);
fleet_gauge!(
    /// `ix.fleet.host.temperature`.
    FLEET_TEMPERATURE, "ix.fleet.host.temperature", "Cel",
    "Highest CPU package or Tctl temperature; absent unless the host profile expects it and it reads."
);
fleet_gauge!(
    /// `ix.fleet.host.capacity_slots`.
    FLEET_CAPACITY_SLOTS, "ix.fleet.host.capacity_slots", "{slot}",
    "Operator-entered capacity slots; absent when none is entered."
);
fleet_gauge!(
    /// `ix.fleet.host.capacity_configured`.
    FLEET_CAPACITY_CONFIGURED, "ix.fleet.host.capacity_configured", "1",
    "1 when capacity is entered, 0 otherwise; always exported."
);
fleet_gauge!(
    /// `ix.fleet.host.native_windows_agents`.
    FLEET_NATIVE_WINDOWS, "ix.fleet.host.native_windows_agents", "{process}",
    "On a WSL host, native Windows agent, herdr or build processes seen; nonzero is a defect."
);
fleet_gauge!(
    /// `ix.fleet.probe_ok`.
    FLEET_PROBE_OK, "ix.fleet.probe_ok", "1",
    "Source health, 1 or 0, for each source the host profile expects."
);
fleet_gauge!(
    /// `ix.fleet.probe_unavailable`.
    FLEET_PROBE_UNAVAILABLE, "ix.fleet.probe_unavailable", "1",
    "Value 1 for a source the host profile declares absent on this host; not a failure."
);

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
    (&HEALTH_SNAPSHOT_READ_FAILURES, &["snapshot"]),
    (&FLEET_LANE_STATE, &["host", "lane", "state"]),
    (&FLEET_LANE_LAST_ACTIVITY, &["host", "lane"]),
    (&FLEET_LANE_BLOCKED, &["host", "lane", "reason"]),
    (&FLEET_LANE_DROPPED, &["host"]),
    (&FLEET_BUSY_RATIO, &["host"]),
    (&FLEET_MEMORY_USED, &["host"]),
    (&FLEET_MEMORY_TOTAL, &["host"]),
    (&FLEET_TEMPERATURE, &["host"]),
    (&FLEET_CAPACITY_SLOTS, &["host"]),
    (&FLEET_CAPACITY_CONFIGURED, &["host"]),
    (&FLEET_NATIVE_WINDOWS, &["host"]),
    (&FLEET_PROBE_OK, &["host", "source"]),
    (&FLEET_PROBE_UNAVAILABLE, &["host", "source"]),
];
