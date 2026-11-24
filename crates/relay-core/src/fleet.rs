//! The fleet lane registry (FR-069): each watcher leases its host's lane list,
//! and the fleet registry is the union of the unexpired lists and the relay
//! registrations. A host whose lease lapsed keeps its lanes, marked stale.

use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};

use crate::MachineName;

/// How long a published lane list is valid, as the fenced herdr directory is.
pub const LANE_LEASE: Duration = Duration::from_secs(90);

/// How often a watcher refreshes its lane list.
pub const LANE_REFRESH: Duration = Duration::from_secs(30);

/// The most lanes one host list holds.
pub const MAX_HOST_LANES: usize = 2_000;

/// One host's lane list as stored: addresses `server:workspace:tab`, stamped
/// with the relay clock when written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostLaneList {
    /// The publishing host.
    pub machine: MachineName,
    /// Relay-clock Unix milliseconds of the last refresh.
    pub refreshed_ms: i64,
    /// The host's lane addresses.
    pub lanes: Vec<String>,
}

impl HostLaneList {
    /// Whether the lease has run out at relay-clock `now_ms`.
    #[must_use]
    pub fn expired(&self, now_ms: i64) -> bool {
        let lease = i64::try_from(LANE_LEASE.as_millis()).unwrap_or(i64::MAX);
        now_ms.saturating_sub(self.refreshed_ms) >= lease
    }
}

/// One lane of the fleet registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetLane {
    /// `server:workspace:tab`.
    pub address: String,
    /// Listed only by a host whose lease has expired, and by no registration.
    pub stale: bool,
}

/// The union of the unexpired `lists` and the `registrations` (lane addresses of
/// relay registrations), with lanes only an expired list names kept and marked
/// stale. Sorted by address; a lane leaves only when no source lists it.
#[must_use]
pub fn fleet_registry(
    lists: &[HostLaneList],
    registrations: &[String],
    now_ms: i64,
) -> Vec<FleetLane> {
    let mut lanes: BTreeMap<&str, bool> = BTreeMap::new();
    for address in registrations {
        lanes.insert(address, false);
    }
    for list in lists {
        let stale = list.expired(now_ms);
        for address in &list.lanes {
            let entry = lanes.entry(address).or_insert(stale);
            *entry = *entry && stale;
        }
    }
    lanes
        .into_iter()
        .map(|(address, stale)| FleetLane {
            address: address.to_owned(),
            stale,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(machine: &str, refreshed_ms: i64, lanes: &[&str]) -> HostLaneList {
        HostLaneList {
            machine: machine.parse().unwrap(),
            refreshed_ms,
            lanes: lanes.iter().map(|l| (*l).to_owned()).collect(),
        }
    }

    /// Tracing: TC-069-16
    /// ACs: FR-069-AC-16
    #[test]
    fn tc_069_16_registry_is_the_union_and_an_expired_host_stays_listed_as_stale() {
        let lists = [
            list("a", 100_000, &["a:w:1", "a:w:2"]),
            list("b", 100_000, &["b:w:1"]),
            list("c", 5_000, &["c:w:1", "a:w:2"]),
        ];
        let regs = vec!["b:w:1".to_owned(), "d:w:9".to_owned()];
        // c refreshed at 5s: at 100s its 90s lease has run out.
        let at = 100_000;
        let reg = fleet_registry(&lists, &regs, at);
        let view: Vec<(&str, bool)> = reg.iter().map(|l| (l.address.as_str(), l.stale)).collect();
        assert_eq!(
            view,
            [
                ("a:w:1", false),
                ("a:w:2", false), // also named by expired c: a live list wins
                ("b:w:1", false),
                ("c:w:1", true), // kept, stale, not dropped
                ("d:w:9", false),
            ]
        );
        // A refresh restores c.
        let lists = [
            list("a", 100_000, &["a:w:1", "a:w:2"]),
            list("b", 100_000, &["b:w:1"]),
            list("c", 99_000, &["c:w:1"]),
        ];
        let reg = fleet_registry(&lists, &regs, at);
        assert!(reg.iter().all(|l| !l.stale));
        assert_eq!(reg.len(), 5);
    }

    /// Tracing: TC-069-16
    /// ACs: FR-069-AC-16
    #[test]
    fn tc_069_16_the_lease_is_ninety_seconds_and_refreshed_about_every_thirty() {
        let l = list("a", 0, &[]);
        assert!(!l.expired(89_999));
        assert!(l.expired(90_000));
        assert_eq!(LANE_REFRESH.as_secs() * 3, LANE_LEASE.as_secs());
    }
}
