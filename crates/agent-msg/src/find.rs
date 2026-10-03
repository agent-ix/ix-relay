//! `agent-msg find`: orchestrates `relay_ops::find`'s live herdr gathering
//! across this machine and every enabled saved herdr machine, joins it
//! against the bus registry, filters by an optional query, and renders it
//! (AGE-2018). The join and filter logic themselves are pure and tested in
//! `relay_ops::find`; this module is the herdr/Redis-calling glue plus
//! plain-text/JSON rendering.

use std::time::Duration;

use relay_core::{MachineName, Relay};
use relay_ops::find::{self, FindHerdr, FoundAgent, MachineView, ProcessError, RealFindHerdr};

use crate::{BoxError, retry::Redial};

/// Maximum live rows included in the machine-readable directory snapshot.
pub(crate) const MAX_DIRECTORY_ROWS: usize = 500;

/// Timeout for each herdr call `find` makes, local or remote: bounded so
/// one unreachable saved machine can't hang the whole command. 10s (not the
/// original 3s): a remote machine's calls go over SSH, and `find` (per
/// [`relay_ops::find::gather_machine_view`]) now makes exactly 3 of them —
/// AGE-2018 PR review, finding 3: at 3s per call, `find` kept skipping hub
/// outright; each machine is still queried in parallel with every other
/// (see [`run`]), so this only raises the ceiling for one slow machine, not
/// the whole command's wall-clock time.
const HERDR_TIMEOUT: Duration = Duration::from_secs(10);

/// One machine `find` couldn't query, and why — printed as a note rather
/// than failing the whole command.
pub struct SkippedMachine {
    /// The machine's label (or this machine's own name, for a local failure).
    pub machine: String,
    /// Why it was skipped.
    pub reason: String,
}

/// What `find` gathered: joined rows (already filtered by the query, if
/// any) and any machines it had to skip.
pub struct FindReport {
    /// The matching rows.
    pub rows: Vec<FoundAgent>,
    /// Machines that errored or timed out and were skipped.
    pub skipped: Vec<SkippedMachine>,
    /// Herdr and watcher state for each machine queried by this snapshot.
    pub machines: Vec<MachineStatus>,
}

/// Reachability and watcher freshness for one configured Herdr machine.
#[derive(Debug, serde::Serialize)]
pub struct MachineStatus {
    /// Herdr's machine label.
    pub machine: String,
    /// Whether Herdr returned a live view from this machine.
    pub herdr: &'static str,
    /// Whether its relay watcher heartbeat is current and healthy.
    pub watcher: &'static str,
    /// Most recent watcher tick, when one is known.
    pub last_tick_ms: Option<i64>,
    /// Why Herdr could not read this machine, when applicable.
    pub detail: Option<String>,
}

/// Stable, bounded JSON envelope for directory integrations.
#[derive(Debug, serde::Serialize)]
pub struct FindReportJson<'a> {
    /// Live Herdr agents, joined only to exact verified relay registrations.
    pub rows: Vec<FoundJson<'a>>,
    /// Reachability and freshness for all queried machines.
    pub machines: &'a [MachineStatus],
    /// Whether every configured machine answered Herdr.
    pub complete: bool,
    /// Whether more live rows matched than the snapshot includes.
    pub truncated: bool,
}

/// Builds the bounded stable JSON payload used by the Herdr directory.
#[must_use]
pub fn report_json(report: &FindReport) -> FindReportJson<'_> {
    FindReportJson {
        rows: report
            .rows
            .iter()
            .take(MAX_DIRECTORY_ROWS)
            .map(FoundJson::from)
            .collect(),
        machines: &report.machines,
        complete: report.skipped.is_empty(),
        truncated: report.rows.len() > MAX_DIRECTORY_ROWS,
    }
}

/// Gathers live herdr state from this machine and every enabled saved
/// machine (queried in parallel, each herdr call bounded by
/// [`HERDR_TIMEOUT`]), joins it against the bus registry, and filters by
/// `query` (empty matches everything).
///
/// # Errors
///
/// Propagates a Redis failure ([`relay_core::Error`]) reading the registry
/// or alias table, after retrying a timed-out read (see [`crate::retry`]). A
/// herdr failure on any one machine does not fail the whole call — it's
/// recorded in [`FindReport::skipped`] instead.
pub fn run(
    relay: &mut Relay,
    redial: &Redial,
    this_machine: &MachineName,
    query: &str,
) -> Result<FindReport, BoxError> {
    let herdr = RealFindHerdr::new();
    let mut views: Vec<MachineView> = Vec::new();
    let mut skipped = Vec::new();

    match find::gather_machine_view(&herdr, None, this_machine, HERDR_TIMEOUT) {
        Ok(view) => views.push(view),
        Err(e) => skipped.push(SkippedMachine {
            machine: this_machine.to_string(),
            reason: e.to_string(),
        }),
    }

    let saved_machines = match herdr.machine_list(HERDR_TIMEOUT) {
        Ok(list) => list.into_iter().filter(|m| m.enabled).collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("agent-msg find: could not list saved herdr machines: {e}");
            skipped.push(SkippedMachine {
                machine: "<saved machine list>".to_owned(),
                reason: e.to_string(),
            });
            Vec::new()
        }
    };

    let results: Vec<(String, Result<MachineView, ProcessError>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = saved_machines
            .iter()
            .map(|m| {
                let herdr = &herdr;
                let label = m.label.clone();
                scope.spawn(move || {
                    // A saved machine whose label is no machine name cannot be
                    // matched against registrations: skipped, with the reason.
                    let result = match MachineName::new(&label) {
                        Ok(name) => {
                            find::gather_machine_view(herdr, Some(&label), &name, HERDR_TIMEOUT)
                        }
                        Err(e) => Err(ProcessError::BadOutput {
                            program: "herdr".to_owned(),
                            reason: e.to_string(),
                        }),
                    };
                    (label, result)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| {
                    (
                        "<unknown>".to_owned(),
                        Err(ProcessError::Spawn {
                            program: "herdr".to_owned(),
                            reason: "gathering that machine's herdr state panicked".to_owned(),
                        }),
                    )
                })
            })
            .collect()
    });
    for (label, result) in results {
        match result {
            Ok(view) => views.push(view),
            Err(e) => skipped.push(SkippedMachine {
                machine: label,
                reason: e.to_string(),
            }),
        }
    }

    // Only reads, so a timed-out one is retried on a fresh connection (after the
    // herdr gathering above, which is not repeated).
    let (list, aliases, heartbeats) = crate::retry::read(relay, redial, |r| {
        let known = views
            .iter()
            .map(|view| view.machine.clone())
            .chain(skipped.iter().filter_map(|row| row.machine.parse().ok()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        Ok((r.agents()?, r.all_aliases()?, r.heartbeats(&known)?))
    })?;
    let registrations =
        list.into_registrations_reporting(|skipped| eprintln!("agent-msg find: {skipped}"));
    let rows = find::join(&views, &registrations, &aliases)
        .into_iter()
        .filter(|row| find::matches_query(row, query))
        .collect();

    let machines = machine_statuses(&views, &skipped, &heartbeats);
    Ok(FindReport {
        rows,
        skipped,
        machines,
    })
}

fn machine_statuses(
    views: &[MachineView],
    skipped: &[SkippedMachine],
    heartbeats: &relay_core::Heartbeats,
) -> Vec<MachineStatus> {
    let mut labels = views
        .iter()
        .map(|view| view.machine.to_string())
        .chain(skipped.iter().map(|row| row.machine.clone()))
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();

    labels
        .into_iter()
        .map(|machine| {
            let detail = skipped
                .iter()
                .find(|row| row.machine == machine)
                .map(|row| bounded_reason(&row.reason));
            let herdr = if detail.is_some() {
                "unavailable"
            } else {
                "live"
            };
            let machine_name = machine.parse::<MachineName>().ok();
            let heartbeat = machine_name
                .as_ref()
                .and_then(|name| heartbeats.found.iter().find(|row| &row.machine == name));
            let marker = machine_name
                .as_ref()
                .and_then(|name| heartbeats.last_ticks.get(name).copied());
            let (watcher, last_tick_ms) = match heartbeat.map(|row| &row.read) {
                Some(relay_core::health::HeartbeatRead::Live(heartbeat)) => {
                    let age = crate::now_ms().saturating_sub(heartbeat.last_tick_ms);
                    let state = if !heartbeat.redis_ok
                        || !heartbeat.herdr_ok
                        || heartbeat.herdr_blocks_delivery
                    {
                        "degraded"
                    } else if relay_core::health::tick_is_stale(age, heartbeat.sweep_secs) {
                        "stale"
                    } else {
                        "fresh"
                    };
                    (state, Some(heartbeat.last_tick_ms))
                }
                Some(relay_core::health::HeartbeatRead::Undecodable) => ("unreadable", marker),
                None if heartbeats.truncated == Some(relay_core::ScanCut::Deadline) => {
                    ("unknown", marker)
                }
                None if marker.is_some() => ("stale", marker),
                None if machine_name.is_some() => ("missing", None),
                None => ("unknown", None),
            };
            MachineStatus {
                machine,
                herdr,
                watcher,
                last_tick_ms,
                detail,
            }
        })
        .collect()
}

fn bounded_reason(reason: &str) -> String {
    relay_core::text::neutralise(reason)
        .chars()
        .take(240)
        .collect()
}

/// One plain-text line for `row`: its `server:workspace[:tab[:pane]]` address first
/// (`-` when herdr gave no workspace label), then the bus name and detail.
#[must_use]
pub fn render_line(row: &FoundAgent) -> String {
    let address = row
        .address
        .as_ref()
        .map_or_else(|| "-".to_owned(), ToString::to_string);
    let name = row
        .bus_name
        .as_ref()
        .map_or_else(|| "-".to_owned(), |n| n.as_str().to_owned());
    let aliases = if row.aliases.is_empty() {
        String::new()
    } else {
        let names = row
            .aliases
            .iter()
            .map(relay_core::AgentName::as_str)
            .collect::<Vec<_>>();
        format!(" aka {}", names.join(", "))
    };
    let workspace = row.workspace_label.as_deref().unwrap_or("-");
    let tab = row.tab_label.as_deref().unwrap_or("-");
    let session = row.session_name.as_deref().unwrap_or("-");
    let bus_flag = if row.on_bus { "" } else { "  [not on bus]" };
    format!(
        "{address}  {name}{aliases}  {}  {workspace}/{tab}  {}  {session}  {}{bus_flag}",
        row.machine, row.pane_id, row.herdr_status
    )
}

/// The `agent-msg find --json` object for one row. The struct is the
/// schema: adding or renaming a field changes the pinned field list in this
/// module's tests.
#[derive(Debug, serde::Serialize)]
pub struct FoundJson<'a> {
    /// The `server:workspace[:tab[:pane]]` address, when the row has one.
    address: Option<String>,
    /// The bus name, when the row is on the bus.
    bus_name: Option<&'a str>,
    /// Every alias pointing at the bus name.
    aliases: Vec<&'a str>,
    /// The machine hosting the pane.
    machine: &'a str,
    /// The herdr workspace label.
    workspace_label: Option<&'a str>,
    /// The herdr tab label.
    tab_label: Option<&'a str>,
    /// The herdr pane id.
    pane_id: &'a str,
    /// The herdr session name.
    session_name: Option<&'a str>,
    /// The pane's herdr agent status.
    herdr_status: &'a str,
    /// Whether the pane is registered on the bus.
    on_bus: bool,
    /// The registration's last refresh time in Unix milliseconds.
    last_seen_ms: Option<i64>,
}

impl<'a> From<&'a FoundAgent> for FoundJson<'a> {
    fn from(row: &'a FoundAgent) -> Self {
        Self {
            address: row.address.as_ref().map(ToString::to_string),
            bus_name: row.bus_name.as_ref().map(relay_core::AgentName::as_str),
            aliases: row
                .aliases
                .iter()
                .map(relay_core::AgentName::as_str)
                .collect(),
            machine: row.machine.as_str(),
            workspace_label: row.workspace_label.as_deref(),
            tab_label: row.tab_label.as_deref(),
            pane_id: row.pane_id.as_str(),
            session_name: row.session_name.as_deref(),
            herdr_status: &row.herdr_status,
            on_bus: row.on_bus,
            last_seen_ms: row.last_seen_ms,
        }
    }
}

/// What `find` ends its output with — "what to give `send`": each row that is
/// on the bus, by its `server:workspace[:tab[:pane]]` address, or by bus name when
/// it has no address or another listed row shares it (an address `send`
/// would refuse as ambiguous).
#[must_use]
pub fn sendable_targets(rows: &[FoundAgent]) -> Vec<String> {
    let shared = relay_core::duplicated(rows.iter().filter_map(|r| r.address.as_ref()));
    rows.iter()
        .filter_map(|r| {
            let name = r.bus_name.as_ref()?;
            let unique_address = r.address.as_ref().filter(|a| !shared.contains(a));
            Some(unique_address.map_or_else(|| name.to_string(), ToString::to_string))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    fn row() -> FoundAgent {
        FoundAgent {
            bus_name: Some("qsl-lead".parse().unwrap()),
            aliases: vec!["qsl".parse().unwrap()],
            machine: "node-a".parse().unwrap(),
            address: Some("node-a:qsl:main".parse().unwrap()),
            workspace_label: Some("qsl".into()),
            tab_label: None,
            pane_id: "p1".parse().unwrap(),
            tab_id: "t1".parse().unwrap(),
            workspace_id: "w1".parse().unwrap(),
            session_id: None,
            session_name: Some("s".into()),
            herdr_status: "idle".into(),
            on_bus: true,
            last_seen_ms: Some(1),
        }
    }

    /// Tracing: TC-008-07
    /// ACs: FR-008-AC-7
    ///
    /// `find --json` is byte-identical to what `main` printed when the machine and
    /// pane were bare strings (literals captured from `main`, AGE-2027 phase B).
    #[test]
    fn found_json_is_byte_identical_to_main() {
        assert_eq!(
            serde_json::to_string(&FoundJson::from(&row())).unwrap(),
            r#"{"address":"node-a:qsl:main","bus_name":"qsl-lead","aliases":["qsl"],"machine":"node-a","workspace_label":"qsl","tab_label":null,"pane_id":"p1","session_name":"s","herdr_status":"idle","on_bus":true,"last_seen_ms":1}"#
        );
        let off_bus = FoundAgent {
            bus_name: None,
            aliases: vec![],
            machine: "hub".parse().unwrap(),
            address: None,
            workspace_label: None,
            tab_label: Some("t".into()),
            pane_id: "w1:p2".parse().unwrap(),
            tab_id: "w1:t1".parse().unwrap(),
            workspace_id: "w1".parse().unwrap(),
            session_id: None,
            session_name: None,
            herdr_status: "working".into(),
            on_bus: false,
            last_seen_ms: None,
        };
        assert_eq!(
            serde_json::to_string(&FoundJson::from(&off_bus)).unwrap(),
            r#"{"address":null,"bus_name":null,"aliases":[],"machine":"hub","workspace_label":null,"tab_label":"t","pane_id":"w1:p2","session_name":null,"herdr_status":"working","on_bus":false,"last_seen_ms":null}"#
        );
    }

    /// `find --json` emits exactly these eleven fields, absent ones as `null`.
    /// Trace: FR-050-AC-3
    #[test]
    fn found_json_pins_the_field_list() {
        let row = row();
        assert_eq!(
            serde_json::to_value(FoundJson::from(&row)).unwrap(),
            json!({
                "address": "node-a:qsl:main",
                "bus_name": "qsl-lead",
                "aliases": ["qsl"],
                "machine": "node-a",
                "workspace_label": "qsl",
                "tab_label": null,
                "pane_id": "p1",
                "session_name": "s",
                "herdr_status": "idle",
                "on_bus": true,
                "last_seen_ms": 1,
            })
        );
    }

    fn heartbeat(machine: &str, last_tick_ms: i64) -> relay_core::health::MachineHeartbeat {
        use relay_core::health::{HeartbeatRead, MachineHeartbeat, WatcherHeartbeat};

        MachineHeartbeat {
            machine: machine.parse().unwrap(),
            read: HeartbeatRead::Live(Box::new(WatcherHeartbeat {
                machine: machine.parse().unwrap(),
                pid: 1,
                version: "test".into(),
                started_at_ms: last_tick_ms,
                last_tick_ms,
                last_sweep_ms: None,
                sweep_secs: 15,
                herdr_ok: true,
                herdr_problem: None,
                herdr_blocks_delivery: false,
                redis_ok: true,
                leads_watched: 1,
                otlp: None,
                last_error: None,
            })),
        }
    }

    /// Trace: FR-050-AC-2
    #[test]
    fn report_distinguishes_fresh_stale_and_unreachable_machines() {
        let views = ["fresh", "stale"]
            .into_iter()
            .map(|machine| MachineView {
                machine: machine.parse().unwrap(),
                agents: vec![],
                workspaces: vec![],
                tabs: vec![],
                dropped: 0,
            })
            .collect::<Vec<_>>();
        let now = crate::now_ms();
        let heartbeats = relay_core::Heartbeats {
            found: vec![heartbeat("fresh", now), heartbeat("stale", 1)],
            truncated: None,
            last_ticks: BTreeMap::from([("unreachable".parse().unwrap(), 1)]),
            unreadable: 0,
        };
        let skipped = [SkippedMachine {
            machine: "unreachable".into(),
            reason: "ssh failed\u{1b}[31m".into(),
        }];

        let report = machine_statuses(&views, &skipped, &heartbeats);
        assert_eq!(report[0].machine, "fresh");
        assert_eq!(report[0].watcher, "fresh");
        assert_eq!(report[1].machine, "stale");
        assert_eq!(report[1].watcher, "stale");
        assert_eq!(report[2].machine, "unreachable");
        assert_eq!(report[2].herdr, "unavailable");
        assert_eq!(report[2].watcher, "stale");
        assert!(!report[2].detail.as_ref().unwrap().contains('\u{1b}'));
    }

    /// Trace: FR-050-AC-2
    #[test]
    fn report_keeps_unreadable_degraded_missing_and_unread_scans_distinct() {
        use relay_core::health::{HeartbeatRead, MachineHeartbeat};

        let names = ["degraded", "missing", "unreadable"];
        let views = names
            .iter()
            .map(|machine| MachineView {
                machine: machine.parse().unwrap(),
                agents: vec![],
                workspaces: vec![],
                tabs: vec![],
                dropped: 0,
            })
            .collect::<Vec<_>>();
        let mut degraded = heartbeat("degraded", crate::now_ms());
        if let HeartbeatRead::Live(record) = &mut degraded.read {
            record.redis_ok = false;
        }
        let heartbeats = relay_core::Heartbeats {
            found: vec![
                degraded,
                MachineHeartbeat {
                    machine: "unreadable".parse().unwrap(),
                    read: HeartbeatRead::Undecodable,
                },
            ],
            truncated: None,
            last_ticks: BTreeMap::new(),
            unreadable: 1,
        };

        let report = machine_statuses(&views, &[], &heartbeats);
        let states = report
            .iter()
            .map(|machine| (machine.machine.as_str(), machine.watcher))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(states["degraded"], "degraded");
        assert_eq!(states["missing"], "missing");
        assert_eq!(states["unreadable"], "unreadable");

        let deadline_health = relay_core::Heartbeats {
            truncated: Some(relay_core::ScanCut::Deadline),
            ..relay_core::Heartbeats::default()
        };
        let deadline_view = [MachineView {
            machine: "deadline".parse().unwrap(),
            agents: vec![],
            workspaces: vec![],
            tabs: vec![],
            dropped: 0,
        }];
        let deadline = machine_statuses(&deadline_view, &[], &deadline_health);
        assert_eq!(deadline[0].watcher, "unknown");
    }

    /// Trace: FR-050-AC-3
    #[test]
    fn report_json_has_stable_top_level_fields_and_marks_incomplete_machines() {
        let report = FindReport {
            rows: vec![row()],
            skipped: vec![SkippedMachine {
                machine: "hub".into(),
                reason: "SSH unavailable".into(),
            }],
            machines: vec![MachineStatus {
                machine: "hub".into(),
                herdr: "unavailable",
                watcher: "unknown",
                last_tick_ms: None,
                detail: Some("SSH unavailable".into()),
            }],
        };
        let payload = serde_json::to_value(report_json(&report)).unwrap();
        assert_eq!(
            payload
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["complete", "machines", "rows", "truncated"]
        );
        assert_eq!(payload["complete"], false);
        assert_eq!(payload["machines"][0]["herdr"], "unavailable");
        assert_eq!(payload["rows"][0]["last_seen_ms"], 1);
        assert_eq!(payload["truncated"], false);
    }
}
