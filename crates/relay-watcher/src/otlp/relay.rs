//! Relay health series: `ix.relay.*`, from data the watcher and the store already
//! have. Low cardinality by construction, and never message text.
//!
//! - What the watcher itself did (`ix.relay.messages`, the tick histogram) is
//!   counted in memory by [`Stats`] as it happens, cumulative since the watcher
//!   started.
//! - What Redis holds (inbox depth and age, in-flight entries, dead letters,
//!   hook failures) is read once per push by [`read`]: two pipelined requests, no
//!   key scan.
//!
//! `lead` is an attribute only where it is bounded: at most [`MAX_LEADS`] leads
//! get their own series and the rest are folded into `lead=other`, with the
//! number folded reported through [`Registry::note_dropped`] (and so in the
//! heartbeat and `doctor`).

use std::{collections::BTreeMap, time::Duration};

use relay_core::{AgentName, MachineName, Registration, Relay};

use super::{
    registry::{Attrs, HistogramData, Registry, label_value},
    series::{
        DEADLETTER_ENTRIES, HEALTH_SNAPSHOT_READ_FAILURES, HOOK_FAILURES, INBOX_DEPTH,
        INBOX_OLDEST_AGE, INFLIGHT_ENTRIES, MESSAGES, REGISTRATIONS, TICK_BOUNDS, TICK_DURATION,
    },
};

/// The most leads that get a series of their own per instrument.
pub const MAX_LEADS: usize = 50;
/// The most distinct `(stage, error_kind)` pairs of hook failures.
pub const MAX_HOOK_GROUPS: usize = 64;
/// The most hook health entries read per push (the stream's own cap).
const HOOK_ENTRIES: usize = 1_000;

/// What the watcher counted itself, since it started.
#[derive(Debug, Clone)]
pub struct Stats {
    delivered: u64,
    requeued: u64,
    redelivered: u64,
    dead_lettered: u64,
    tick: HistogramData,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            delivered: 0,
            requeued: 0,
            redelivered: 0,
            dead_lettered: 0,
            tick: HistogramData::new(TICK_BOUNDS),
        }
    }
}

impl Stats {
    /// Messages this watcher typed into a pane.
    pub fn delivered(&mut self, n: usize) {
        self.delivered = self.delivered.saturating_add(n as u64);
    }

    /// Messages a rejected prompt put back at the front of the inbox.
    pub fn requeued(&mut self, n: usize) {
        self.requeued = self.requeued.saturating_add(n as u64);
    }

    /// Messages an expired lease returned to an inbox, marked as redeliveries.
    pub fn redelivered(&mut self, n: usize) {
        self.redelivered = self.redelivered.saturating_add(n as u64);
    }

    /// Messages an expired lease that had spent its redeliveries moved to the
    /// dead letters.
    pub fn dead_lettered(&mut self, n: usize) {
        self.dead_lettered = self.dead_lettered.saturating_add(n as u64);
    }

    /// One tick's duration.
    pub fn tick(&mut self, took: Duration) {
        self.tick.record(took.as_secs_f64());
    }

    /// Adds these to `registry`.
    pub fn fill(&self, registry: &mut Registry) {
        for (outcome, n) in [
            ("delivered", self.delivered),
            ("requeued", self.requeued),
            ("redelivered", self.redelivered),
            ("dead_lettered", self.dead_lettered),
        ] {
            registry.sum(&MESSAGES, Attrs::new().with("outcome", outcome), n);
        }
        registry.histogram(&TICK_DURATION, Attrs::new(), &self.tick);
    }
}

/// One local lead's numbers from Redis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LeadRow {
    depth: u64,
    oldest_age_secs: u64,
    in_flight: u64,
    dead: u64,
}

/// What one push read from Redis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Health {
    leads: BTreeMap<String, LeadRow>,
    /// Hook failures of this machine since the watcher started, by
    /// `(stage, error_kind)`.
    hook_failures: BTreeMap<(String, String), u64>,
    registrations: u64,
}

/// Failed reads of each source used to build one health snapshot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ReadFailures {
    /// Failed per-lead or whole-source queue overview reads.
    pub(super) lead_overviews: u64,
    /// Failed whole-source newest hook-health reads.
    pub(super) hook_health_newest: u64,
}

impl ReadFailures {
    pub(super) fn fill(&self, registry: &mut Registry) {
        for (snapshot, count) in [
            ("lead_overviews", self.lead_overviews),
            ("hook_health_newest", self.hook_health_newest),
        ] {
            registry.sum(
                &HEALTH_SNAPSHOT_READ_FAILURES,
                Attrs::new().with("snapshot", snapshot),
                count,
            );
        }
    }
}

/// A hook health record's `at_ms`, `stage` and `error_kind`, as far as they read.
/// The record is written by another process: nothing else of it is used, and its
/// strings are made safe before they become attributes.
fn hook_failure(record: &str) -> Option<(i64, String, String)> {
    let value: serde_json::Value = serde_json::from_str(record).ok()?;
    Some((
        value.get("at_ms")?.as_i64()?,
        label_value(value.get("stage")?.as_str()?),
        label_value(value.get("error_kind")?.as_str()?),
    ))
}

/// Reads what the store holds about `regs`' leads on `machine`, as of `now_ms`: one
/// pipelined request for every local lead's queues (inbox length and oldest message,
/// in-flight and dead-letter lengths; no key scan) and one for the newest hook
/// failures, so at most `PUSH_REQUESTS` requests. A read that fails leaves
/// its series out; the first error is returned beside what was read, and the caller
/// decides what to do with it (the tick only logs it).
pub(super) fn read(
    relay: &mut Relay,
    machine: &MachineName,
    regs: &[Registration],
    started_ms: i64,
    now_ms: i64,
) -> (Health, ReadFailures, Option<relay_core::Error>) {
    let local: Vec<&AgentName> = regs
        .iter()
        .filter(|r| r.pane_on(machine).is_some())
        .map(|r| &r.name)
        .collect();
    let mut health = Health {
        registrations: local.len() as u64,
        ..Health::default()
    };
    let mut failed = None;
    let mut read_failures = ReadFailures::default();
    match relay.lead_overviews(&local) {
        Ok(overviews) => {
            for (name, overview) in local.iter().zip(overviews) {
                let Ok(overview) = overview else {
                    read_failures.lead_overviews = read_failures.lead_overviews.saturating_add(1);
                    continue;
                };
                // A lead has a row only from a read that succeeded: a failed read
                // leaves its series out, never a zero that reads as a drained inbox.
                health.leads.insert(
                    name.as_str().to_owned(),
                    LeadRow {
                        depth: overview.len as u64,
                        oldest_age_secs: overview.oldest_sent_at_ms.map_or(0, |sent| {
                            u64::try_from(now_ms.saturating_sub(sent) / 1000).unwrap_or(0)
                        }),
                        in_flight: overview.in_flight as u64,
                        dead: overview.dead as u64,
                    },
                );
            }
        }
        Err(e) => {
            read_failures.lead_overviews = 1;
            failed = Some(e);
        }
    }
    match relay.hook_health_newest(HOOK_ENTRIES) {
        Ok(stream) => {
            for entry in stream.entries.iter().filter(|e| e.machine == *machine) {
                let Some((at_ms, stage, kind)) = hook_failure(&entry.record) else {
                    continue;
                };
                if at_ms < started_ms {
                    continue;
                }
                let key = if health
                    .hook_failures
                    .contains_key(&(stage.clone(), kind.clone()))
                    || health.hook_failures.len() < MAX_HOOK_GROUPS
                {
                    (stage, kind)
                } else {
                    ("other".to_owned(), "other".to_owned())
                };
                *health.hook_failures.entry(key).or_default() += 1;
            }
        }
        Err(e) => {
            read_failures.hook_health_newest = 1;
            failed = failed.or(Some(e));
        }
    }
    (health, read_failures, failed)
}

impl Health {
    /// Adds these to `registry`: per-lead gauges (folding leads past
    /// [`MAX_LEADS`] into `lead=other`), the hook failure counts and the
    /// registration gauge for `machine`.
    pub fn fill(&self, registry: &mut Registry, machine: &MachineName) {
        let mut other = LeadRow::default();
        let mut folded = 0u64;
        for (index, (lead, row)) in self.leads.iter().enumerate() {
            if index < MAX_LEADS {
                emit_lead(registry, lead, row);
            } else {
                folded += 1;
                other.depth = other.depth.saturating_add(row.depth);
                other.oldest_age_secs = other.oldest_age_secs.max(row.oldest_age_secs);
                other.in_flight = other.in_flight.saturating_add(row.in_flight);
                other.dead = other.dead.saturating_add(row.dead);
            }
        }
        if folded > 0 {
            emit_lead(registry, "other", &other);
            registry.note_dropped(folded);
        }
        for ((stage, kind), n) in &self.hook_failures {
            registry.gauge(
                &HOOK_FAILURES,
                Attrs::new().with("stage", stage).with("error_kind", kind),
                i64::try_from(*n).unwrap_or(i64::MAX),
            );
        }
        registry.gauge(
            &REGISTRATIONS,
            Attrs::new().with("machine", machine.as_str()),
            i64::try_from(self.registrations).unwrap_or(i64::MAX),
        );
    }
}

fn emit_lead(registry: &mut Registry, lead: &str, row: &LeadRow) {
    let attrs = || Attrs::new().with("lead", lead);
    let gauge = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
    registry.gauge(&INBOX_DEPTH, attrs(), gauge(row.depth));
    registry.gauge(&INBOX_OLDEST_AGE, attrs(), gauge(row.oldest_age_secs));
    registry.gauge(&INFLIGHT_ENTRIES, attrs(), gauge(row.in_flight));
    registry.gauge(&DEADLETTER_ENTRIES, attrs(), gauge(row.dead));
}

#[cfg(test)]
impl Health {
    /// A health reading for a test: one row per `(lead, depth, oldest_age_secs,
    /// batches, dead)`.
    pub(crate) fn for_test(
        rows: &[(&str, u64, u64, u64, u64)],
        hooks: &[((&str, &str), u64)],
    ) -> Self {
        Self {
            leads: rows
                .iter()
                .map(|(lead, depth, age, batches, dead)| {
                    (
                        (*lead).to_owned(),
                        LeadRow {
                            depth: *depth,
                            oldest_age_secs: *age,
                            in_flight: *batches,
                            dead: *dead,
                        },
                    )
                })
                .collect(),
            hook_failures: hooks
                .iter()
                .map(|((s, k), n)| (((*s).to_owned(), (*k).to_owned()), *n))
                .collect(),
            registrations: rows.len() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp::registry::{Point, Value};

    fn hub() -> MachineName {
        "hub".parse().unwrap()
    }

    fn value_of(r: &Registry, name: &str, key: &str, want: &str) -> Option<i64> {
        r.points().iter().find_map(|p: &Point| {
            (p.instrument.name == name
                && p.attrs.pairs().iter().any(|(k, v)| *k == key && v == want))
            .then_some(match p.value {
                Value::Int(v) => Some(v),
                _ => None,
            })?
        })
    }

    /// Tracing: TC-045-12
    /// ACs: FR-045-AC-12
    #[test]
    fn tc_045_12_leads_past_the_cap_are_folded_into_other_and_counted() {
        let rows: Vec<(String, u64, u64, u64, u64)> = (0..MAX_LEADS + 3)
            .map(|n| (format!("lead-{n:03}"), 2, n as u64, 1, 1))
            .collect();
        let refs: Vec<(&str, u64, u64, u64, u64)> = rows
            .iter()
            .map(|(l, a, b, c, d)| (l.as_str(), *a, *b, *c, *d))
            .collect();
        let mut r = Registry::new(2, 1);
        Health::for_test(&refs, &[]).fill(&mut r, &hub());
        let leads: std::collections::BTreeSet<_> = r
            .points()
            .iter()
            .filter(|p| p.instrument.name == "ix.relay.inbox.depth")
            .map(|p| p.attrs.pairs()[0].1.clone())
            .collect();
        assert_eq!(leads.len(), MAX_LEADS + 1, "the cap and one `other`");
        assert!(leads.contains("other"));
        assert!(!leads.contains(&format!("lead-{:03}", MAX_LEADS)));
        // Depth sums, oldest age is the largest, and the drop is reported.
        assert_eq!(
            value_of(&r, "ix.relay.inbox.depth", "lead", "other"),
            Some(6)
        );
        assert_eq!(
            value_of(&r, "ix.relay.inbox.oldest_age", "lead", "other"),
            Some(MAX_LEADS as i64 + 2)
        );
        assert_eq!(r.dropped(), 3);
    }

    /// Tracing: TC-045-12
    /// ACs: FR-045-AC-12
    #[test]
    fn tc_045_12_hook_failure_records_are_read_leniently_and_their_text_is_made_safe() {
        assert_eq!(
            hook_failure(r#"{"at_ms":5,"stage":"connect","error_kind":"timeout","extra":1}"#),
            Some((5, "connect".into(), "timeout".into()))
        );
        let hostile =
            hook_failure("{\"at_ms\":5,\"stage\":\"a\\nb\\u001b\",\"error_kind\":\"x\"}").unwrap();
        assert_eq!(hostile.1, "a\u{FFFD}b\u{FFFD}");
        for bad in [
            "",
            "{",
            "[]",
            r#"{"at_ms":"5","stage":"a","error_kind":"b"}"#,
            r#"{"at_ms":1,"stage":1,"error_kind":"b"}"#,
            r#"{"at_ms":1}"#,
        ] {
            assert_eq!(hook_failure(bad), None, "{bad:?}");
        }
    }

    fn test_relay(test: &str) -> Option<Relay> {
        let url = std::env::var("IX_RELAY_TEST_URL").ok()?;
        let relay = Relay::connect(&url, Duration::from_secs(5)).expect("connect to test redis");
        Some(relay.with_prefix(format!("test:{}:{test}:", std::process::id())))
    }

    /// The push read is the newest failures first, so past the stream's 1,000 entries
    /// the gauge still counts the latest ones; and it is exactly the two pipelined
    /// requests, with each lead's queue lengths (inbox, oldest, in-flight, dead) from
    /// the keys themselves. Redis lane.
    ///
    /// Tracing: TC-045-23
    /// ACs: FR-045-AC-23, FR-045-AC-22
    #[test]
    fn tc_045_23_the_hook_gauge_counts_the_newest_failures_past_a_thousand_entries() {
        let Some(mut relay) = test_relay("hook-newest") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let hub = hub();
        let now_ms = 1_700_000_000_000_i64;
        let entry = |stage: &str| relay_core::HookHealthEntry {
            machine: hub.clone(),
            lead: None,
            record: format!(r#"{{"at_ms":{now_ms},"stage":"{stage}","error_kind":"timeout"}}"#),
        };
        let mut batch: Vec<_> = (0..1_000).map(|_| entry("old")).collect();
        batch.extend((0..100).map(|_| entry("new")));
        for chunk in batch.chunks(100) {
            relay.push_hook_health(chunk).unwrap();
        }
        // The precondition that makes this test discriminating: the stream holds
        // more than one read of 1,000 (its cap is approximate).
        let held = relay.hook_health(usize::MAX / 2).unwrap().entries.len();
        assert!(held > 1_000, "the stream was trimmed to {held} entries");
        let mut lead = reg_for("a-lead", "hub", "w1:p1");
        lead.last_seen_ms = now_ms;
        let (health, _, failed) = read(&mut relay, &hub, &[lead], 0, now_ms);
        assert!(failed.is_none(), "{failed:?}");
        assert_eq!(
            health
                .hook_failures
                .get(&("new".to_owned(), "timeout".to_owned())),
            Some(&100),
            "the newest failures are counted: {:?}",
            health.hook_failures
        );
    }

    fn reg_for(name: &str, machine: &str, pane: &str) -> Registration {
        Registration {
            name: name.parse().unwrap(),
            machine: machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: None,
            cwd: None,
            herdr_pane: Some(pane.parse().unwrap()),
            place: None,
            last_seen_ms: 0,
        }
    }

    /// The queue lengths of a lead come from its own keys: inbox depth, in-flight
    /// entries and dead letters, with the oldest message's age. Redis lane.
    ///
    /// Tracing: TC-045-12
    /// ACs: FR-045-AC-12
    #[test]
    fn tc_045_12_the_lead_rows_come_from_the_leads_own_keys() {
        let Some(mut relay) = test_relay("lead-rows") else {
            eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
            return;
        };
        let lead = reg_for("a-lead", "hub", "w1:p1");
        relay.register(&lead).unwrap();
        for n in 0..3 {
            let msg = relay_core::Message {
                from: "sender".parse().unwrap(),
                to: "a-lead".parse().unwrap(),
                text: format!("m{n}"),
                sent_at_ms: 1_000,
            };
            relay.send(&msg).unwrap();
        }
        let (health, _, failed) = read(&mut relay, &hub(), std::slice::from_ref(&lead), 0, 61_000);
        assert!(failed.is_none(), "{failed:?}");
        let row = &health.leads["a-lead"];
        assert_eq!(
            (row.depth, row.oldest_age_secs, row.in_flight, row.dead),
            (3, 60, 0, 0)
        );
        // Two popped and not acknowledged are in flight; one is left in the inbox.
        relay
            .pop_inbox_budget(
                &lead.name,
                std::num::NonZeroUsize::new(2).unwrap(),
                1_000_000,
                relay_core::Claim::new(relay_core::Popper::Hook),
            )
            .unwrap();
        let (health, _, _) = read(&mut relay, &hub(), &[lead], 0, 61_000);
        let row = &health.leads["a-lead"];
        assert_eq!((row.depth, row.in_flight, row.dead), (1, 2, 0));
    }

    #[test]
    fn the_stats_fill_every_outcome_even_at_zero_and_the_tick_histogram() {
        let mut stats = Stats::default();
        stats.delivered(3);
        stats.requeued(1);
        stats.tick(Duration::from_millis(20));
        let mut r = Registry::new(2, 1);
        stats.fill(&mut r);
        assert_eq!(
            value_of(&r, "ix.relay.messages", "outcome", "delivered"),
            Some(3)
        );
        assert_eq!(
            value_of(&r, "ix.relay.messages", "outcome", "requeued"),
            Some(1)
        );
        assert_eq!(
            value_of(&r, "ix.relay.messages", "outcome", "dead_lettered"),
            Some(0)
        );
        assert_eq!(
            value_of(&r, "ix.relay.messages", "outcome", "redelivered"),
            Some(0)
        );
        assert!(
            r.points()
                .iter()
                .any(|p| p.instrument.name == "ix.relay.watcher.tick.duration")
        );
    }
}
