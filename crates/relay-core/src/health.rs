//! What the watcher publishes about itself, in types both binaries share.
//!
//! Two records, both written by `relay-watcher` and read by `agent-msg doctor`
//! and `agent-msg send`:
//!
//! - [`WatcherHeartbeat`], the bus record `<prefix>health:watcher:<machine>`
//!   (FR-028), refreshed on every tick with a short expiry. A missing key means
//!   no watcher has ticked on that machine recently: an expired heartbeat and a
//!   watcher that never ran look the same, on purpose.
//! - [`WatcherFile`], the local `~/.ix/relay/state/watcher.json`, readable with
//!   Redis down.
//!
//! A reader of a heartbeat tolerates members it does not know: a newer watcher
//! may add some, and an older `agent-msg` must still see that it is alive.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ids::MachineName;

/// The file name of the local state file, inside [`crate::config::state_dir`].
pub const WATCHER_FILE_NAME: &str = "watcher.json";

/// Slack, in milliseconds, added to two sweep intervals before a heartbeat's last
/// tick reads as stale: for a tick that is slow, and for clocks that differ a
/// little.
pub const TICK_STALE_SLACK_MS: i64 = 10_000;

/// How far ahead of the reader's clock a heartbeat's last tick may be (milliseconds)
/// and still read as "just now": two machines' clocks differ a little.
pub const TICK_FUTURE_TOLERANCE_MS: i64 = 5_000;

/// Whether a heartbeat whose last tick was `age_ms` ago, from a watcher that sweeps
/// every `sweep_secs`, is stale: a live watcher ticks at least every sweep, so
/// older than two sweeps plus [`TICK_STALE_SLACK_MS`] is a watcher that stopped
/// while its key has not yet expired (three sweeps). The one rule `agent-msg send`
/// and `agent-msg doctor` share.
#[must_use]
pub fn tick_is_stale(age_ms: i64, sweep_secs: u64) -> bool {
    let sweep_ms = i64::try_from(sweep_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
    age_ms
        > sweep_ms
            .saturating_mul(2)
            .saturating_add(TICK_STALE_SLACK_MS)
}

/// The shortest a heartbeat lives, whatever the sweep interval (FR-028).
pub const HEARTBEAT_MIN_TTL: Duration = Duration::from_secs(30);

/// How long a heartbeat written by a watcher that sweeps every `sweep` lives:
/// three sweep intervals, and at least [`HEARTBEAT_MIN_TTL`], so one slow tick
/// does not read as a dead watcher and a dead one is noticed within a minute or
/// so.
#[must_use]
pub fn heartbeat_ttl(sweep: Duration) -> Duration {
    sweep.saturating_mul(3).max(HEARTBEAT_MIN_TTL)
}

/// The longest text kept in [`WatcherHeartbeat::herdr_problem`], in characters.
pub const MAX_PROBLEM_CHARS: usize = 200;

/// How a watcher's last failed tick failed, as a stable code (never message
/// text: a heartbeat is read by other machines and rendered to a terminal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatErrorKind {
    /// Redis was unreachable, or a call to it timed out or dropped.
    RedisUnreachable,
    /// The tick failed on a working connection (a script refused, a key of the
    /// wrong type, an ACL).
    PollFailed,
    /// A code a newer watcher wrote that this binary does not know.
    #[serde(other)]
    Unknown,
}

/// The longest `target` or `reason` kept in an [`OtlpStatus`], in characters.
pub const MAX_OTLP_TEXT_CHARS: usize = 64;

/// What the exporter has done since the watcher started (FR-045): pushes the
/// collector accepted, pushes that failed, snapshots dropped unsent (a full queue,
/// or a backoff) and data points a collector said it rejected. Absent from a
/// record when all four are zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtlpCounters {
    /// Pushes the collector accepted.
    #[serde(default)]
    pub sent: u64,
    /// Pushes that failed, for any reason.
    #[serde(default)]
    pub failed: u64,
    /// Snapshots never sent.
    #[serde(default)]
    pub dropped_snapshots: u64,
    /// Data points a collector reported as rejected.
    #[serde(default)]
    pub rejected_points: u64,
}

impl OtlpCounters {
    /// Whether all four are zero.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// What the watcher's OTLP metrics push is doing (FR-045), as a heartbeat carries
/// it. A collector that is down is a warning, never a reason for the watcher to
/// stop. `target` is `host:port` only (no credential, path or query) and `reason`
/// is a short code, never message text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OtlpStatus {
    /// No endpoint is configured: nothing is sent.
    #[default]
    Disabled,
    /// Nothing is sent, because an inherited generic OpenTelemetry setting was
    /// ignored (an https or gRPC endpoint meant for another emitter). `note` names
    /// the variable and the reason, never a value.
    Ignoring {
        /// `VARIABLE ignored: reason`.
        note: String,
    },
    /// Pushes reach the collector at `target` (or none has been attempted yet).
    Sending {
        /// The collector's `host:port`.
        target: String,
        /// Series the last snapshot left out or folded into `lead=other` to stay
        /// inside its bounds (FR-045); absent when none.
        #[serde(default, skip_serializing_if = "is_zero")]
        dropped: u64,
        /// What the exporter has done since the watcher started.
        #[serde(default, skip_serializing_if = "OtlpCounters::is_empty")]
        counters: OtlpCounters,
    },
    /// The last push failed and the watcher is backing off.
    Failing {
        /// The collector's `host:port`.
        target: String,
        /// Why, as a code such as `connect_failed`, `timeout`, `http_503`,
        /// `rejected_points` or `stuck`.
        reason: String,
        /// What the exporter has done since the watcher started.
        #[serde(default, skip_serializing_if = "OtlpCounters::is_empty")]
        counters: OtlpCounters,
    },
    /// A state a newer watcher wrote that this binary does not know.
    #[serde(other)]
    Unknown,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Reads the `otlp` member without letting a malformed one fail the record: a
/// state this binary cannot decode is [`OtlpStatus::Unknown`].
fn lenient_otlp<'de, D>(deserializer: D) -> Result<Option<OtlpStatus>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(raw.map(|value| serde_json::from_value(value).unwrap_or(OtlpStatus::Unknown)))
}

/// The most recent failed tick, as a heartbeat carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatError {
    /// What kind of failure.
    pub kind: HeartbeatErrorKind,
    /// When it happened, Unix milliseconds.
    pub at_ms: i64,
}

/// `<prefix>health:watcher:<machine>`: one machine's watcher, as of its last
/// good tick. Written with the expiry [`heartbeat_ttl`] gives, so the key
/// itself is the liveness signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatcherHeartbeat {
    /// The machine this watcher serves, normalised.
    pub machine: MachineName,
    /// The watcher's process id.
    pub pid: u32,
    /// The watcher's version (`CARGO_PKG_VERSION`).
    pub version: String,
    /// When the process started, Unix milliseconds.
    pub started_at_ms: i64,
    /// When this record was written: the time of the last good tick.
    pub last_tick_ms: i64,
    /// When a backup sweep last ran, `None` before the first.
    pub last_sweep_ms: Option<i64>,
    /// The sweep interval the expiry was derived from, in seconds.
    pub sweep_secs: u64,
    /// Whether herdr passed its version and shape check
    /// ([`WatcherHeartbeat::herdr_problem`] says what failed when not).
    pub herdr_ok: bool,
    /// What the herdr check found wrong, bounded to [`MAX_PROBLEM_CHARS`]
    /// characters. Text from a binary's output: render it neutralised.
    pub herdr_problem: Option<String>,
    /// Whether the watcher types into no pane because herdr's output is in a shape
    /// ix-relay cannot read (FR-036). Absent from a record an older watcher wrote,
    /// which reads as `false`.
    #[serde(default)]
    pub herdr_blocks_delivery: bool,
    /// Whether every tick since the previous heartbeat reached Redis: `false`
    /// when a connection error was ridden out in between, a flap the expiry
    /// alone would have hidden.
    pub redis_ok: bool,
    /// How many registered leads with a pane this machine's tick looked at.
    pub leads_watched: u32,
    /// The state of the OTLP metrics push. `None` in a record an older watcher
    /// wrote, which did not say: not the same as disabled. A member that does
    /// not decode reads as [`OtlpStatus::Unknown`], and never makes the whole
    /// heartbeat unreadable.
    #[serde(default, deserialize_with = "lenient_otlp")]
    pub otlp: Option<OtlpStatus>,
    /// The most recent failed tick, kept after recovery.
    pub last_error: Option<HeartbeatError>,
}

/// What reading one machine's heartbeat key found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeartbeatRead {
    /// A record that decoded.
    Live(Box<WatcherHeartbeat>),
    /// A key that is there but not a heartbeat this binary can read.
    Undecodable,
}

/// One machine's heartbeat key, as [`crate::Relay::heartbeats`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineHeartbeat {
    /// The machine, from the key name.
    pub machine: MachineName,
    /// What the key held.
    pub read: HeartbeatRead,
}

/// `~/.ix/relay/state/watcher.json`: what the watcher was last doing, readable
/// with Redis down. Rewritten atomically on every poll.
///
/// Read by `agent-msg doctor`, which may be older than the watcher that wrote it,
/// so a member this binary does not know is ignored, not refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatcherFile {
    /// Process id of the watcher that wrote this.
    pub pid: u32,
    /// Unix milliseconds when this watcher process started.
    pub started_at_ms: i64,
    /// Unix milliseconds of the last poll that succeeded; `None` before the
    /// first one.
    pub last_ok_tick_ms: Option<i64>,
    /// The most recent failed poll; kept after recovery so a past outage stays
    /// visible.
    pub last_error: Option<LastError>,
}

/// One failed poll, as recorded in [`WatcherFile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastError {
    /// Unix milliseconds when it failed.
    pub at_ms: i64,
    /// The error, as displayed.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WatcherHeartbeat {
        WatcherHeartbeat {
            machine: "hub".parse().unwrap(),
            pid: 7,
            version: "0.1.0".into(),
            started_at_ms: 1,
            last_tick_ms: 2,
            last_sweep_ms: Some(3),
            sweep_secs: 15,
            herdr_ok: true,
            herdr_problem: None,
            herdr_blocks_delivery: false,
            redis_ok: true,
            leads_watched: 2,
            otlp: Some(OtlpStatus::Failing {
                target: "collector:4318".into(),
                reason: "timeout".into(),
                counters: OtlpCounters::default(),
            }),
            last_error: Some(HeartbeatError {
                kind: HeartbeatErrorKind::RedisUnreachable,
                at_ms: 4,
            }),
        }
    }

    /// Every member of the heartbeat is a documented wire member: adding or
    /// renaming one fails here and asks for the spec (FR-028) to change too.
    #[test]
    fn the_heartbeat_has_exactly_the_documented_fields() {
        let value = serde_json::to_value(sample()).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "herdr_blocks_delivery",
                "herdr_ok",
                "herdr_problem",
                "last_error",
                "last_sweep_ms",
                "last_tick_ms",
                "leads_watched",
                "machine",
                "otlp",
                "pid",
                "redis_ok",
                "started_at_ms",
                "sweep_secs",
                "version",
            ]
        );
        assert_eq!(
            value["last_error"],
            serde_json::json!({"kind": "redis_unreachable", "at_ms": 4})
        );
    }

    /// A newer watcher's `watcher.json`, with a member this binary has never
    /// heard of (also inside `last_error`), still reads: an older `doctor` must
    /// not report a healthy newer watcher's file as unreadable.
    #[test]
    fn a_newer_state_file_with_extra_members_still_reads() {
        let raw = r#"{"pid":7,"started_at_ms":1,"last_ok_tick_ms":2,"added_later":true,
            "last_error":{"at_ms":3,"message":"m","kind":"future"}}"#;
        let file: WatcherFile = serde_json::from_str(raw).unwrap();
        assert_eq!((file.pid, file.last_ok_tick_ms), (7, Some(2)));
        assert_eq!(file.last_error.unwrap().at_ms, 3);
    }

    /// A heartbeat an older watcher wrote has no `otlp` member: it reads as
    /// `None`, not as disabled. A state a newer watcher invents, and a known
    /// state with a malformed body, read as unknown and do not fail the record.
    ///
    /// Tracing: TC-045-08
    /// ACs: FR-045-AC-8
    #[test]
    fn a_heartbeat_without_otlp_reads_as_none_and_an_unusable_state_as_unknown() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value.as_object_mut().unwrap().remove("otlp");
        let read: WatcherHeartbeat = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(read.otlp, None);
        for odd in [
            serde_json::json!({"state": "from_the_future", "x": 1}),
            serde_json::json!({"state": "sending"}),
            serde_json::json!({"state": "failing", "target": "c:1"}),
            serde_json::json!({"state": "sending", "target": 5}),
            serde_json::json!("sending"),
            serde_json::json!(7),
        ] {
            value["otlp"] = odd.clone();
            let read: WatcherHeartbeat = serde_json::from_value(value.clone())
                .unwrap_or_else(|e| panic!("{odd} made the heartbeat unreadable: {e}"));
            assert_eq!(read.otlp, Some(OtlpStatus::Unknown), "{odd}");
        }
        // A known state reads as itself.
        value["otlp"] = serde_json::json!({"state": "sending", "target": "c:1", "extra": 1});
        let read: WatcherHeartbeat = serde_json::from_value(value).unwrap();
        assert_eq!(
            read.otlp,
            Some(OtlpStatus::Sending {
                target: "c:1".into(),
                dropped: 0,
                counters: OtlpCounters::default(),
            })
        );
    }

    /// The exporter's counters ride in `sending` and `failing` only when not all
    /// zero, read back whole, and a record without them reads as zeros; the
    /// `ignoring` state carries its note.
    ///
    /// Tracing: TC-045-21
    /// ACs: FR-045-AC-21
    #[test]
    fn tc_045_21_the_exporter_counters_and_the_ignoring_note_round_trip() {
        let counters = OtlpCounters {
            sent: 3,
            failed: 2,
            dropped_snapshots: 1,
            rejected_points: 9,
        };
        let sending = OtlpStatus::Sending {
            target: "c:1".into(),
            dropped: 0,
            counters,
        };
        let json = serde_json::to_value(&sending).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"state":"sending","target":"c:1","counters":
                {"sent":3,"failed":2,"dropped_snapshots":1,"rejected_points":9}})
        );
        assert_eq!(serde_json::from_value::<OtlpStatus>(json).unwrap(), sending);
        let quiet = serde_json::to_value(OtlpStatus::Sending {
            target: "c:1".into(),
            dropped: 0,
            counters: OtlpCounters::default(),
        })
        .unwrap();
        assert_eq!(quiet, serde_json::json!({"state":"sending","target":"c:1"}));
        let ignoring = OtlpStatus::Ignoring {
            note: "OTEL_EXPORTER_OTLP_ENDPOINT ignored: https is not supported".into(),
        };
        let json = serde_json::to_value(&ignoring).unwrap();
        assert_eq!(
            serde_json::from_value::<OtlpStatus>(json).unwrap(),
            ignoring
        );
    }

    #[test]
    fn the_stale_rule_is_two_sweeps_plus_ten_seconds() {
        // Sweep 15 s: stale beyond 40 s.
        assert!(!tick_is_stale(40_000, 15));
        assert!(tick_is_stale(40_001, 15));
        assert!(tick_is_stale(i64::MAX, 15));
        assert!(!tick_is_stale(-1, 15));
    }

    #[test]
    fn the_expiry_is_three_sweeps_and_never_under_thirty_seconds() {
        assert_eq!(
            heartbeat_ttl(Duration::from_secs(20)),
            Duration::from_secs(60)
        );
        assert_eq!(heartbeat_ttl(Duration::from_secs(1)), HEARTBEAT_MIN_TTL);
        assert_eq!(heartbeat_ttl(Duration::MAX), Duration::MAX);
    }

    #[test]
    fn a_newer_heartbeat_with_extra_members_and_an_unknown_error_kind_still_reads() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value["added_later"] = serde_json::json!(1);
        value["last_error"]["kind"] = serde_json::json!("a_kind_from_the_future");
        let read: WatcherHeartbeat = serde_json::from_value(value).unwrap();
        assert_eq!(
            read.last_error.map(|e| e.kind),
            Some(HeartbeatErrorKind::Unknown)
        );
    }
}
