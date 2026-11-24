//! The per-host lane lists of the fleet registry against real Redis.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test fixtures")]
use relay_core::fleet::fleet_registry;
use relay_test_support::Bus;

/// Tracing: TC-069-16
/// ACs: FR-069-AC-16
#[test]
fn tc_069_16_three_hosts_publish_lane_lists_and_the_registry_is_their_union() {
    let Some(mut bus) = Bus::connect("fleet-lane-lists") else {
        return;
    };
    for (host, lanes) in [
        ("draco", vec!["draco:QSL:plan".to_owned()]),
        (
            "titan",
            vec!["titan:IR:code".to_owned(), "titan:IR:plan".to_owned()],
        ),
        ("luna", vec!["luna:QSL:plan".to_owned()]),
    ] {
        bus.relay
            .publish_lane_list(&host.parse().unwrap(), &lanes)
            .unwrap();
    }
    let read = bus.relay.lane_lists().unwrap();
    assert_eq!(read.lists.len(), 3);
    assert_eq!(read.unreadable, 0);
    let registry = fleet_registry(&read.lists, &["terra:EA:code".to_owned()], read.now_ms);
    assert_eq!(registry.len(), 5);
    assert!(registry.iter().all(|l| !l.stale));
    // Ninety seconds on, with no refresh, every published lane is kept and stale;
    // the registration still lists its own lane.
    let later = fleet_registry(
        &read.lists,
        &["terra:EA:code".to_owned()],
        read.now_ms + 90_000,
    );
    assert_eq!(later.len(), 5);
    assert_eq!(later.iter().filter(|l| l.stale).count(), 4);
}
