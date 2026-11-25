//! The hook feed in the collector: lane state from events, `hook_events` health,
//! compile attribution, focus info, the configured windows and the hook counts.

use std::time::{Duration, Instant};

use ix_trace_rs::trace;

use super::{feed::Windows, fixtures::*, *};
use crate::otlp::registry::Value;

const MIN: i64 = 60;

fn registry_of(c: &Collector, now: Instant) -> Registry {
    let mut r = Registry::new(2, 1);
    c.fill(&mut r, now);
    r
}

fn value(r: &Registry, name: &str, want: &[(&str, &str)]) -> Option<i64> {
    r.points().iter().find_map(|p| {
        let hit = p.instrument.name == name
            && want
                .iter()
                .all(|(k, v)| p.attrs.pairs().iter().any(|(pk, pv)| pk == k && pv == v));
        match (&p.value, hit) {
            (Value::Int(v), true) => Some(*v),
            _ => None,
        }
    })
}

fn state_of(r: &Registry, lane: &str) -> Vec<&'static str> {
    ["working", "idle", "blocked", "stuck", "ended"]
        .into_iter()
        .filter(|s| value(r, "ix.fleet.lane.state", &[("lane", lane), ("state", s)]).is_some())
        .collect()
}

fn no_events(r: &Registry, lane: &str) -> bool {
    value(r, "ix.fleet.lane.no_events", &[("lane", lane)]) == Some(1)
}

fn hook_events(r: &Registry) -> Option<i64> {
    value(r, "ix.fleet.probe_ok", &[("source", "hook_events")])
}

/// One event of a fixture lane: kind, seconds ago, `last_class`.
type Ev<'a> = (&'a str, i64, Option<&'a str>);

fn lane_with(address: &str, herdr: &str) -> Lane {
    Lane {
        address: address.into(),
        pane: Some("w1:p1".parse().unwrap()),
        harness: Harness::Claude,
        herdr_status: Some(herdr.into()),
    }
}

/// Lanes first seen at `t0`, the feed read at `t0 + elapsed` with `events` per lane.
fn collector_at(
    lanes: &[(&str, &str)],
    events: &[(&str, Vec<Ev>)],
    windows: Windows,
    elapsed: Duration,
) -> (Collector, Instant) {
    let mut c =
        Collector::with_windows(host(), Profile::default(), sources(None), INTERVAL, windows);
    let t0 = Instant::now();
    c.set_lanes(t0, lanes.iter().map(|(a, h)| lane_with(a, h)).collect());
    let now = t0 + elapsed;
    let records = events
        .iter()
        .map(|(a, e)| ((*a).to_owned(), record(e, "/w/x", "b")))
        .collect();
    c.set_activity_at(now, NOW_MS, Ok(records), &mut |_| {});
    (c, now)
}

#[trace("FR-045-AC-30")]
#[test]
fn tc_045_30_the_newest_evidence_names_each_lanes_state() {
    let (c, now) = collector_at(
        &[
            ("hub:S:none", "idle"),
            ("hub:S:blocked", "working"),
            ("hub:S:question", "idle"),
            ("hub:S:open1h", "idle"),
            ("hub:S:open3h", "idle"),
            ("hub:S:old11m", "idle"),
            ("hub:S:ended", "working"),
            ("hub:S:stopped", "working"),
        ],
        &[
            (
                "hub:S:blocked",
                vec![("post_tool", 60, None), ("blocked", 20, None)],
            ),
            ("hub:S:question", vec![("stop", 20, Some("question"))]),
            ("hub:S:open1h", vec![("pre_tool", 60 * MIN, None)]),
            ("hub:S:open3h", vec![("pre_tool", 180 * MIN, None)]),
            ("hub:S:old11m", vec![("post_tool", 11 * MIN, None)]),
            (
                "hub:S:ended",
                vec![("prompt", 600, None), ("session_end", 30, None)],
            ),
            (
                "hub:S:stopped",
                vec![("prompt", 600, None), ("stop", 30, Some("done"))],
            ),
        ],
        Windows::default(),
        Duration::ZERO,
    );
    let r = registry_of(&c, now);
    assert_eq!(state_of(&r, "hub:S:none"), ["idle"]);
    assert_eq!(state_of(&r, "hub:S:blocked"), ["blocked"]);
    assert_eq!(
        value(
            &r,
            "ix.fleet.lane.blocked",
            &[("lane", "hub:S:blocked"), ("reason", "permission")]
        ),
        Some(1)
    );
    assert_eq!(state_of(&r, "hub:S:question"), ["blocked"]);
    assert_eq!(
        value(
            &r,
            "ix.fleet.lane.blocked",
            &[("lane", "hub:S:question"), ("reason", "question")]
        ),
        Some(1)
    );
    assert_eq!(state_of(&r, "hub:S:open1h"), ["working"]);
    assert_eq!(state_of(&r, "hub:S:open3h"), ["stuck"]);
    assert_eq!(state_of(&r, "hub:S:old11m"), ["idle"]);
    assert_eq!(state_of(&r, "hub:S:ended"), ["ended"]);
    assert_eq!(
        state_of(&r, "hub:S:stopped"),
        ["idle"],
        "the stop is newer than herdr's working"
    );
    assert_eq!(
        value(
            &r,
            "ix.fleet.lane.last_activity",
            &[("lane", "hub:S:old11m")]
        ),
        Some(11 * MIN)
    );
}

#[trace("FR-069-AC-8")]
#[test]
fn tc_069_08_a_newer_hook_event_beats_an_older_pane_class_and_an_older_one_loses() {
    let (mut c, now) = collector_at(
        &[("hub:S:a", "idle")],
        &[("hub:S:a", vec![("prompt", 5, None)])],
        Windows::default(),
        Duration::ZERO,
    );
    // The dialog was read 20 s before the prompt.
    let read = now.checked_sub(Duration::from_secs(20)).unwrap();
    c.record(Source::PaneTail, Outcome::Ok, &mut |_| {});
    c.classes
        .insert("hub:S:a".into(), (PaneClass::Dialog, read));
    let r = registry_of(&c, now);
    assert_eq!(state_of(&r, "hub:S:a"), ["working"]);
    assert!(value(&r, "ix.fleet.lane.blocked", &[("lane", "hub:S:a")]).is_none());
}

#[trace("FR-070-AC-9")]
#[test]
fn tc_070_09_silent_non_idle_lanes_count_unhealthy_and_quiet_lanes_do_not() {
    let silent = Duration::from_secs(31 * 60);
    // An open pre_tool, herdr working with no event ever, and a prompt with no stop.
    let (c, now) = collector_at(
        &[
            ("hub:S:tool", "idle"),
            ("hub:S:herdr", "working"),
            ("hub:S:prompt", "idle"),
        ],
        &[
            ("hub:S:tool", vec![("pre_tool", 40 * MIN, None)]),
            ("hub:S:prompt", vec![("prompt", 40 * MIN, None)]),
        ],
        Windows::default(),
        silent,
    );
    let r = registry_of(&c, now);
    for lane in ["hub:S:tool", "hub:S:herdr", "hub:S:prompt"] {
        assert!(no_events(&r, lane), "{lane}");
    }
    assert_eq!(hook_events(&r), Some(0));

    // Lanes whose newest event is stop, idle or session_end are quiet.
    let (c, now) = collector_at(
        &[
            ("hub:S:stop", "idle"),
            ("hub:S:idle", "idle"),
            ("hub:S:end", "idle"),
        ],
        &[
            (
                "hub:S:stop",
                vec![
                    ("prompt", 300 * MIN, None),
                    ("stop", 200 * MIN, Some("done")),
                ],
            ),
            ("hub:S:idle", vec![("idle", 200 * MIN, None)]),
            ("hub:S:end", vec![("session_end", 200 * MIN, None)]),
        ],
        Windows::default(),
        silent,
    );
    let r = registry_of(&c, now);
    for lane in ["hub:S:stop", "hub:S:idle", "hub:S:end"] {
        assert!(!no_events(&r, lane), "{lane}");
    }
    assert_eq!(hook_events(&r), Some(1));

    // One lane with a recent event among silent working lanes.
    let (c, now) = collector_at(
        &[
            ("hub:S:live", "idle"),
            ("hub:S:a", "idle"),
            ("hub:S:b", "idle"),
        ],
        &[
            ("hub:S:live", vec![("post_tool", 10, None)]),
            ("hub:S:a", vec![("pre_tool", 40 * MIN, None)]),
            ("hub:S:b", vec![("prompt", 50 * MIN, None)]),
        ],
        Windows::default(),
        silent,
    );
    let r = registry_of(&c, now);
    assert_eq!(hook_events(&r), Some(1));
    assert!(!no_events(&r, "hub:S:live"));
    assert!(no_events(&r, "hub:S:a") && no_events(&r, "hub:S:b"));
}

#[trace("FR-070-AC-9")]
#[test]
fn tc_070_09_an_activity_failure_newer_than_every_lanes_events_reads_0_and_an_older_one_does_not() {
    let (mut c, now) = collector_at(
        &[("hub:S:a", "idle")],
        &[("hub:S:a", vec![("stop", 600, Some("done"))])],
        Windows::default(),
        Duration::ZERO,
    );
    c.set_activity_failure(now, Some(Duration::from_secs(60)));
    c.sample(now, &mut |_| {});
    assert_eq!(hook_events(&registry_of(&c, now)), Some(0));
    c.set_activity_failure(now, Some(Duration::from_secs(3600)));
    c.sample(now, &mut |_| {});
    assert_eq!(hook_events(&registry_of(&c, now)), Some(1));
}

#[trace("FR-070-AC-9")]
#[test]
fn tc_070_09_a_feed_that_cannot_be_read_reads_0_and_flags_no_lane() {
    let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
    let t0 = Instant::now();
    c.set_lanes(t0, vec![lane_with("hub:S:a", "working")]);
    let r = registry_of(&c, t0 + Duration::from_secs(3600));
    assert_eq!(hook_events(&r), Some(0), "not attached yet");
    c.set_activity_at(t0, NOW_MS, Err(ErrorClass::Unreadable), &mut |_| {});
    let r = registry_of(&c, t0 + Duration::from_secs(3600));
    assert_eq!(hook_events(&r), Some(0));
    assert!(!no_events(&r, "hub:S:a"));
}

fn compiling(cwd: &str) -> Sources {
    let mut s = sources(None);
    let cwd = cwd.to_owned();
    s.compilers = Arc::new(move || {
        Ok(vec![Compiler {
            name: "rustc".into(),
            cwd: cwd.clone(),
        }])
    });
    s
}

#[trace("FR-069-AC-5", "FR-069-AC-13", "FR-045-AC-33")]
#[test]
fn tc_069_13_a_compiler_adds_its_seconds_to_the_lane_with_the_longest_cwd() {
    let mut c = Collector::new(
        host(),
        Profile::default(),
        compiling("/w/a/b/src"),
        INTERVAL,
    );
    let t0 = Instant::now();
    c.set_lanes(
        t0,
        vec![
            lane_with("hub:S:outer", "idle"),
            lane_with("hub:S:inner", "idle"),
        ],
    );
    let feed = |c: &mut Collector, at: Instant, outer_cwd: &str| {
        let events = [("stop", 10, Some("done"))];
        c.set_activity_at(
            at,
            NOW_MS,
            Ok(vec![
                ("hub:S:outer".into(), record(&events, outer_cwd, "b")),
                ("hub:S:inner".into(), record(&events, "/w/a/b", "b")),
            ]),
            &mut |_| {},
        );
    };
    feed(&mut c, t0, "/w/a");
    let mut last = 0;
    for tick in 0..4u64 {
        let at = t0 + Duration::from_secs(30 * tick);
        c.sample(at, &mut |_| {});
        let r = registry_of(&c, at);
        let now_secs = value(
            &r,
            "ix.fleet.lane.compile_seconds",
            &[("lane", "hub:S:inner")],
        )
        .unwrap();
        assert!(now_secs >= last, "never decreases");
        last = now_secs;
    }
    let at = t0 + Duration::from_secs(90);
    let r = registry_of(&c, at);
    assert_eq!(
        value(
            &r,
            "ix.fleet.lane.compile_seconds",
            &[("lane", "hub:S:inner")]
        ),
        Some(90)
    );
    assert_eq!(
        value(
            &r,
            "ix.fleet.lane.compile_seconds",
            &[("lane", "hub:S:outer")]
        ),
        Some(0)
    );
    assert_eq!(
        state_of(&r, "hub:S:inner"),
        ["working"],
        "a compiler keeps the lane working"
    );
    assert_eq!(state_of(&r, "hub:S:outer"), ["idle"]);
}

#[trace("FR-069-AC-6")]
#[test]
fn tc_069_06_a_mapping_older_than_six_hours_attributes_nothing() {
    let mut c = Collector::new(host(), Profile::default(), compiling("/w/a"), INTERVAL);
    let t0 = Instant::now();
    c.set_lanes(t0, vec![lane_with("hub:S:a", "idle")]);
    let hours = |h: i64| record(&[("stop", h * 3600, Some("done"))], "/w/a", "b");
    for (age, expect) in [(7, 0), (5, 60)] {
        c.set_activity_at(
            t0,
            NOW_MS,
            Ok(vec![("hub:S:a".into(), hours(age))]),
            &mut |_| {},
        );
        let before = c.compile_ms.get("hub:S:a").copied().unwrap_or(0);
        c.sample(t0 + Duration::from_secs(1000), &mut |_| {});
        c.sample(t0 + Duration::from_secs(1030), &mut |_| {});
        c.sample(t0 + Duration::from_secs(1060), &mut |_| {});
        let gained = c.compile_ms.get("hub:S:a").copied().unwrap_or(0) - before;
        assert_eq!(gained, expect * 1000, "{age} h old");
        c.last_sample = None;
    }
}

#[trace("FR-069-AC-13")]
#[test]
fn tc_069_13_a_lane_beyond_fifty_is_not_exported() {
    let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
    let t0 = Instant::now();
    c.set_lanes(
        t0,
        (0..51)
            .map(|n| lane_with(&format!("hub:W:t{n:02}"), "idle"))
            .collect(),
    );
    let r = registry_of(&c, t0);
    let exported = r
        .points()
        .iter()
        .filter(|p| p.instrument.name == "ix.fleet.lane.compile_seconds")
        .count();
    assert_eq!(exported, 50);
}

#[trace("FR-045-AC-32")]
#[test]
fn tc_045_32_the_config_series_reports_the_windows_and_the_rules_use_them() {
    let events = &[("hub:S:a", vec![("prompt", 120, None)])];
    let (c, now) = collector_at(
        &[("hub:S:a", "idle")],
        events,
        Windows::default(),
        Duration::ZERO,
    );
    let r = registry_of(&c, now);
    for (name, secs) in [
        ("working_window", 600),
        ("no_events_after", 1800),
        ("max_tool_runtime", 7200),
    ] {
        assert_eq!(
            value(&r, "ix.fleet.config", &[("name", name)]),
            Some(secs),
            "{name}"
        );
    }
    assert_eq!(state_of(&r, "hub:S:a"), ["working"]);

    let windows = Windows {
        working_window: Duration::from_secs(60),
        max_tool_runtime: Duration::from_secs(90),
        no_events_after: Duration::from_secs(100),
    };
    let (c, now) = collector_at(&[("hub:S:a", "idle")], events, windows, Duration::ZERO);
    let r = registry_of(&c, now);
    assert_eq!(
        value(&r, "ix.fleet.config", &[("name", "working_window")]),
        Some(60)
    );
    assert_eq!(
        value(&r, "ix.fleet.config", &[("name", "max_tool_runtime")]),
        Some(90)
    );
    assert_eq!(
        value(&r, "ix.fleet.config", &[("name", "no_events_after")]),
        Some(100)
    );
    assert_eq!(
        state_of(&r, "hub:S:a"),
        ["idle"],
        "past the configured working window"
    );
    assert!(
        no_events(&r, "hub:S:a"),
        "past the configured no_events_after"
    );
    let (c, now) = collector_at(
        &[("hub:S:a", "idle")],
        &[("hub:S:a", vec![("pre_tool", 100, None)])],
        windows,
        Duration::ZERO,
    );
    assert_eq!(state_of(&registry_of(&c, now), "hub:S:a"), ["stuck"]);
}

#[trace("FR-045-AC-29")]
#[test]
fn tc_045_29_focus_info_carries_the_ticket_of_the_branch_and_is_empty_without_one() {
    let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
    let t0 = Instant::now();
    c.set_lanes(
        t0,
        vec![lane_with("hub:S:a", "idle"), lane_with("hub:S:b", "idle")],
    );
    c.set_activity_at(
        t0,
        NOW_MS,
        Ok(vec![(
            "hub:S:a".into(),
            record(
                &[("prompt", 5, None)],
                "/w/ix-relay-swm-205",
                "swm-205-fleet-series",
            ),
        )]),
        &mut |_| {},
    );
    let r = registry_of(&c, t0);
    assert_eq!(
        value(
            &r,
            "ix.fleet.lane.focus_info",
            &[
                ("lane", "hub:S:a"),
                ("ticket", "SWM-205"),
                ("branch", "swm-205-fleet-series")
            ]
        ),
        Some(1)
    );
    assert_eq!(
        value(
            &r,
            "ix.fleet.lane.focus_info",
            &[("lane", "hub:S:b"), ("ticket", ""), ("branch", "")]
        ),
        Some(1)
    );
}

#[trace("FR-068-AC-11", "FR-068-AC-15")]
#[test]
fn tc_068_11_the_hook_counts_are_exported_by_reason_and_left_out_when_unreadable() {
    let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
    let t0 = Instant::now();
    c.sample(t0, &mut |_| {});
    let r = registry_of(&c, t0);
    for (reason, n) in [
        ("backoff", 3),
        ("failed", 2),
        ("unregistered", 1),
        ("lock", 4),
        ("write_failed", 5),
    ] {
        assert_eq!(
            value(&r, "ix.relay.hook.activity_skipped", &[("reason", reason)]),
            Some(n)
        );
    }
    c.sources.hook_counts = Arc::new(|| Err(ErrorClass::Parse));
    c.sample(t0 + Duration::from_secs(30), &mut |_| {});
    let r = registry_of(&c, t0);
    assert!(
        r.points()
            .iter()
            .all(|p| p.instrument.name != "ix.relay.hook.activity_skipped")
    );
}

/// A skip count at its cap is exported with a saturated gauge of 1, a count that is not is 0,
/// and a sidecar that cannot be read leaves out only its own series.
#[trace("FR-068-AC-15")]
#[test]
fn tc_068_15_a_full_skip_count_is_saturated_and_an_unreadable_one_is_left_out() {
    let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
    let t0 = Instant::now();
    c.sample(t0, &mut |_| {});
    let sat = |r: &Registry, reason: &str| {
        value(
            r,
            "ix.relay.hook.activity_skip_saturated",
            &[("reason", reason)],
        )
    };
    let r = registry_of(&c, t0);
    assert_eq!(
        (sat(&r, "lock"), sat(&r, "write_failed")),
        (Some(0), Some(0))
    );
    c.sources.hook_counts = Arc::new(|| {
        Ok(Some(HookCounts {
            has_state: true,
            skipped: 1,
            lock_skipped: Some(relay_core::hook_skips::MAX_SKIPS),
            write_failed: None,
            ..HookCounts::default()
        }))
    });
    c.sample(t0 + Duration::from_secs(30), &mut |_| {});
    let r = registry_of(&c, t0);
    assert_eq!(sat(&r, "lock"), Some(1));
    assert_eq!(sat(&r, "write_failed"), None);
    assert_eq!(
        value(
            &r,
            "ix.relay.hook.activity_skipped",
            &[("reason", "write_failed")]
        ),
        None
    );
    assert_eq!(
        value(
            &r,
            "ix.relay.hook.activity_skipped",
            &[("reason", "backoff")]
        ),
        Some(1),
        "the state file's counts stay"
    );
}

/// One sample (which reads every pane tail as plain output, class `none`) and then
/// the push, as production runs them.
fn after_sample(lane: (&str, &str), events: Vec<Ev>, windows: Windows) -> (Collector, Instant) {
    let (mut c, now) = collector_at(&[lane], &[(lane.0, events)], windows, Duration::ZERO);
    c.sample(now, &mut |_| {});
    (c, now)
}

#[trace("FR-045-AC-30")]
#[test]
fn tc_045_30_a_pane_class_of_none_never_overrides_the_hook_verdict_after_a_sample() {
    let lane = ("hub:S:a", "idle");
    let cases: [(&str, Vec<Ev>, &str); 4] = [
        (
            "an open pre_tool past max_tool_runtime",
            vec![("pre_tool", 180 * MIN, None)],
            "stuck",
        ),
        (
            "a session_end",
            vec![("prompt", 600, None), ("session_end", 30, None)],
            "ended",
        ),
        (
            "a recent prompt while herdr is idle",
            vec![("prompt", 20, None)],
            "working",
        ),
        (
            "a question Stop",
            vec![("stop", 20, Some("question"))],
            "blocked",
        ),
    ];
    for (what, events, want) in cases {
        let (c, now) = after_sample(lane, events, Windows::default());
        assert!(
            c.classes.contains_key("hub:S:a"),
            "the sample read the pane"
        );
        let r = registry_of(&c, now);
        assert_eq!(state_of(&r, "hub:S:a"), [want], "{what}");
        if want == "blocked" {
            assert_eq!(
                value(&r, "ix.fleet.lane.blocked", &[("reason", "question")]),
                Some(1)
            );
        }
    }
}

#[trace("FR-045-AC-30")]
#[test]
fn tc_045_30_a_pane_class_with_a_reason_read_after_the_newest_event_blocks_the_lane() {
    let (mut c, now) = collector_at(
        &[("hub:S:a", "idle")],
        &[("hub:S:a", vec![("prompt", 20, None)])],
        Windows::default(),
        Duration::ZERO,
    );
    c.sources.pane_tail = Arc::new(|_, _| Ok("Do you want to proceed?\n".to_owned()));
    c.sample(now, &mut |_| {});
    let r = registry_of(&c, now);
    assert_eq!(state_of(&r, "hub:S:a"), ["blocked"]);
    assert_eq!(
        value(&r, "ix.fleet.lane.blocked", &[("reason", "dialog")]),
        Some(1)
    );
}

#[trace("FR-045-AC-30")]
#[test]
fn tc_045_30_a_lane_with_no_hook_event_is_idle_whatever_herdr_says_and_no_events_marks_it() {
    let silent = Duration::from_secs(31 * 60);
    let (mut c, now) = collector_at(&[("hub:S:a", "working")], &[], Windows::default(), silent);
    c.sample(now, &mut |_| {});
    let r = registry_of(&c, now);
    assert_eq!(state_of(&r, "hub:S:a"), ["idle"]);
    assert!(no_events(&r, "hub:S:a"), "FR-070 still flags it");
}

#[trace("FR-045-AC-29")]
#[test]
fn tc_069_06_focus_info_drops_branch_and_ticket_with_the_mapping_after_six_hours() {
    let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
    let t0 = Instant::now();
    c.set_lanes(t0, vec![lane_with("hub:S:a", "idle")]);
    let focus = |c: &Collector, ticket: &str, branch: &str| {
        value(
            &registry_of(c, t0),
            "ix.fleet.lane.focus_info",
            &[("lane", "hub:S:a"), ("ticket", ticket), ("branch", branch)],
        )
    };
    for (hours, live) in [(5, true), (7, false)] {
        c.set_activity_at(
            t0,
            NOW_MS,
            Ok(vec![(
                "hub:S:a".into(),
                record(
                    &[("stop", hours * 3600, Some("done"))],
                    "/w/x",
                    "swm-205-fleet",
                ),
            )]),
            &mut |_| {},
        );
        assert_eq!(
            focus(&c, "SWM-205", "swm-205-fleet"),
            live.then_some(1),
            "{hours} h"
        );
        assert_eq!(focus(&c, "", ""), (!live).then_some(1), "{hours} h");
    }
}

#[trace("FR-070-AC-11")]
#[test]
fn tc_070_11_a_lane_waiting_on_a_permission_is_quiet_not_unhealthy() {
    let (c, now) = collector_at(
        &[("hub:S:a", "idle"), ("hub:S:b", "idle")],
        &[
            (
                "hub:S:a",
                vec![("prompt", 300 * MIN, None), ("blocked", 200 * MIN, None)],
            ),
            ("hub:S:b", vec![("blocked", 200 * MIN, None)]),
        ],
        Windows::default(),
        Duration::ZERO,
    );
    let r = registry_of(&c, now);
    assert!(!no_events(&r, "hub:S:a") && !no_events(&r, "hub:S:b"));
    assert_eq!(hook_events(&r), Some(1));
    assert_eq!(state_of(&r, "hub:S:a"), ["blocked"]);
}

fn counters_probe(r: &Registry) -> Option<i64> {
    value(r, "ix.fleet.probe_ok", &[("source", "activity_counters")])
}

fn has_skipped(r: &Registry) -> bool {
    r.points()
        .iter()
        .any(|p| p.instrument.name == "ix.relay.hook.activity_skipped")
}

#[trace("FR-070-AC-12")]
#[test]
fn tc_070_12_an_unreadable_counts_file_fails_the_source_once_and_a_missing_one_exports_nothing() {
    let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
    let t0 = Instant::now();
    let mut log = Vec::new();
    c.sources.hook_counts = Arc::new(|| Err(ErrorClass::Parse));
    c.sample(t0, &mut |l| log.push(l));
    c.sample(t0 + Duration::from_secs(30), &mut |l| log.push(l));
    let r = registry_of(&c, t0);
    assert_eq!(counters_probe(&r), Some(0));
    assert!(!has_skipped(&r), "no series, no zeros");
    let lines: Vec<&String> = log
        .iter()
        .filter(|l| l.contains("activity_counters"))
        .collect();
    assert_eq!(lines.len(), 1, "one line per transition: {log:?}");
    assert!(lines[0].contains("parse"));
    // The hooks never ran: the file is missing, which is healthy and exports nothing.
    c.sources.hook_counts = Arc::new(|| Ok(None));
    c.sample(t0 + Duration::from_secs(60), &mut |_| {});
    let r = registry_of(&c, t0);
    assert_eq!(counters_probe(&r), Some(1));
    assert!(!has_skipped(&r));
}
