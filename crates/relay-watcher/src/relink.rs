//! Follows a lead's session to the pane that now hosts it (FR-043, FR-032).
//!
//! A registration names a herdr pane (`herdr_pane`) and the harness session
//! (`session_id`) it registered. herdr's own ids do not change when a workspace,
//! tab or pane is renamed, and a pane keeps its id when it moves to another tab
//! of its workspace. `pane move` to another workspace gives the pane a new id in
//! the new workspace, and it keeps its terminal and its session (measured on
//! herdr 0.9.1, `spec/reviews/herdr-id-stability.md`). The session id is
//! therefore the durable join key, and the pane id is a locator that can go stale.
//!
//! Once per place-refresh period, from the one `agent list` the refresh already
//! reads, [`judge`] decides for each local registration with a session id:
//!
//! - its pane hosts its session: alive, nothing to do;
//! - its pane is gone, or hosts a different session, and exactly one other pane
//!   hosts its session: relink the registration to that pane ([`Relay::relink_pane`]);
//! - its pane hosts a different session and its own is nowhere: the pane was
//!   replaced, recorded for `who` and `send` ([`Relay::mark_pane_replaced`]);
//! - anything else: unknown, and unknown changes nothing. A pane that shows an
//!   agent with no session (the harness has not reported yet), two panes claiming
//!   the session, or a herdr that cannot answer never relinks anything.
//!
//! The relink is a compare-and-set against the whole registration the watcher
//! read, so a lead pruned or registered again in the window stays as it is, and
//! only this machine's registrations are looked at, so another machine's pane ids
//! (which repeat, FR-009) never match.

use std::collections::{HashMap, HashSet};

use relay_core::moves::PaneReplaced;
use relay_core::{AgentName, MachineName, PaneId, PaneRef, Registration, Relay, SessionId};

use crate::herdr::Layout;

/// What herdr's agent list says about one registration's pane and session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The registered pane hosts the registered session.
    Alive,
    /// The session is hosted by exactly this other pane, and the registered pane
    /// is gone or hosts a different session.
    Relink(PaneId),
    /// The registered pane hosts a different session and the registered one is on
    /// no pane of the machine.
    Replaced,
    /// Two or more other panes host the session: which one is the lead's cannot
    /// be told, so nothing is relinked.
    Ambiguous,
    /// Nothing can be concluded: the pane shows an agent with no session id, or
    /// neither pane nor session is shown.
    Unknown,
}

/// Decides what to do about a registration naming `pane` with session `session`,
/// given every listed pane of its machine and the session it reports (`None`: an
/// agent whose harness has not reported one). Pure: no herdr, no Redis.
#[must_use]
pub fn judge(
    session: &SessionId,
    pane: &PaneId,
    listed: &HashMap<PaneId, Option<SessionId>>,
) -> Verdict {
    let here = listed.get(pane);
    if here == Some(&Some(session.clone())) {
        return Verdict::Alive;
    }
    // A pane that shows an agent with no session may be the lead itself before
    // its harness reported: it proves nothing either way.
    if matches!(here, Some(None)) {
        return Verdict::Unknown;
    }
    let mut hosts: Vec<&PaneId> = listed
        .iter()
        .filter(|(other, s)| *other != pane && s.as_ref() == Some(session))
        .map(|(other, _)| other)
        .collect();
    match (hosts.len(), here) {
        (1, _) => hosts
            .pop()
            .map_or(Verdict::Unknown, |to| Verdict::Relink(to.clone())),
        (0, Some(Some(_))) => Verdict::Replaced,
        (0, _) => Verdict::Unknown,
        _ => Verdict::Ambiguous,
    }
}

/// Whether Herdr currently confirms the claimed pane and its workspace/tab for
/// this exact harness session. A missing Herdr session id is unknown evidence,
/// not confirmation.
#[must_use]
pub fn pane_has_current_location(reg: &Registration, layout: &Layout) -> bool {
    let (Some(pane), Some(session)) = (reg.herdr_pane.as_ref(), reg.session_id.as_ref()) else {
        return false;
    };
    layout
        .places
        .get(pane)
        .and_then(|place| place.herdr_ids.as_ref())
        .is_some_and(|ids| &ids.session_id == session && &ids.pane == pane)
        && matches!(layout.sessions.get(pane), Some(Some(live)) if live == session)
}

/// What was already reported, so a condition that lasts is logged once per
/// episode, not on every refresh.
#[derive(Debug, Default)]
pub struct Reported {
    /// Leads whose relink is refused for a lasting reason: two panes host the
    /// session, another registration of it holds the target, or the target has no
    /// place.
    trouble: HashSet<AgentName>,
    /// Leads whose replaced-pane marker could not be written, cleared or read.
    marker_trouble: HashSet<AgentName>,
    /// Whether reading the markers is failing.
    marker_read_failing: bool,
}

/// Logs a failure that leaves the connection usable once per lead episode, and
/// returns a connection-level failure so the tick ends and Redis is redialled.
fn once_per_episode(
    seen: &mut HashSet<AgentName>,
    name: &AgentName,
    error: relay_core::Error,
    what: &str,
    log: &mut impl FnMut(String),
) -> relay_core::Result<()> {
    if error.is_connection() {
        return Err(error);
    }
    if seen.insert(name.clone()) {
        log(format!("{name}: {what}: {error}"));
    }
    Ok(())
}

/// Runs the relink pass over the registrations on `machine`, updating the ones it
/// relinked **in `regs`**, and returns their names.
///
/// The caller holds `regs` as the tick's snapshot and as the cached registry: a
/// relinked lead is changed there (pane, place, stamp) and left out of the rest of
/// the tick, so neither the tick nor the cache uses the pane it left, and a lead
/// that was just found alive is never pruned as if it were not.
///
/// A herdr answer that leaves anything unknown changes nothing. A failure of one
/// lead's write is that lead's alone, logged once per episode; only a
/// connection-level Redis error is returned.
///
/// # Errors
///
/// A connection-level Redis error, so the tick ends and the connection is
/// redialled.
pub fn relink_moved(
    relay: &mut Relay,
    layout: &Layout,
    regs: &mut [Registration],
    machine: &MachineName,
    now_ms: i64,
    reported: &mut Reported,
    log: &mut impl FnMut(String),
) -> relay_core::Result<HashSet<AgentName>> {
    let mut relinked = HashSet::new();
    // herdr listed no agent (a restart, or none running): every registration is
    // unknown, and unknown changes nothing, so there is nothing to read.
    if layout.sessions.is_empty() {
        return Ok(relinked);
    }
    let verdicts: Vec<(usize, Verdict)> = regs
        .iter()
        .enumerate()
        .filter(|(_, r)| r.machine == *machine)
        .filter_map(|(i, r)| {
            Some((
                i,
                judge(
                    r.session_id.as_ref()?,
                    r.herdr_pane.as_ref()?,
                    &layout.sessions,
                ),
            ))
        })
        .collect();
    let markers = markers_of_alive(relay, regs, &verdicts, reported, log)?;
    for (i, verdict) in verdicts {
        let reg = regs[i].clone();
        let (name, Some(pane)) = (&reg.name, reg.herdr_pane.as_ref()) else {
            continue;
        };
        // A refusal that ended (the session is home again, or unknown) can be
        // reported again if it comes back; one that lasts is reported once.
        if matches!(
            verdict,
            Verdict::Alive | Verdict::Replaced | Verdict::Unknown
        ) {
            reported.trouble.remove(name);
        }
        match verdict {
            Verdict::Alive => {
                if markers.contains(name) {
                    match relay.clear_pane_replaced(name) {
                        Ok(_) => {
                            reported.marker_trouble.remove(name);
                        }
                        Err(e) => once_per_episode(
                            &mut reported.marker_trouble,
                            name,
                            e,
                            "could not clear its replaced-pane marker",
                            log,
                        )?,
                    }
                }
            }
            Verdict::Relink(to) => {
                if let Some(moved) =
                    relink_one(relay, layout, regs, &reg, &to, now_ms, reported, log)?
                {
                    regs[i].herdr_pane = moved.pane;
                    regs[i].place = moved.new_place;
                    regs[i].last_seen_ms = now_ms.max(reg.last_seen_ms);
                    relinked.insert(name.clone());
                }
            }
            Verdict::Replaced => {
                let found = PaneReplaced {
                    pane: pane.clone(),
                    seen_last_seen_ms: reg.last_seen_ms,
                    since_ms: now_ms,
                };
                match relay.mark_pane_replaced(name, &found) {
                    Ok(wrote) => {
                        reported.marker_trouble.remove(name);
                        if wrote {
                            log(format!(
                                "{name}: its pane {pane} now hosts another session; the \
                                 registration is kept and shown as replaced"
                            ));
                        }
                    }
                    Err(e) => once_per_episode(
                        &mut reported.marker_trouble,
                        name,
                        e,
                        "could not record its replaced pane",
                        log,
                    )?,
                }
            }
            Verdict::Ambiguous => {
                if reported.trouble.insert(name.clone()) {
                    log(format!(
                        "{name}: its session is hosted by more than one pane, so its \
                         registration is not moved"
                    ));
                }
            }
            Verdict::Unknown => {}
        }
    }
    let live: HashSet<&AgentName> = regs
        .iter()
        .filter(|r| r.machine == *machine)
        .map(|r| &r.name)
        .collect();
    reported.trouble.retain(|name| live.contains(name));
    reported.marker_trouble.retain(|name| live.contains(name));
    Ok(relinked)
}

/// The leads with a live pane among `verdicts` that have a replaced-pane marker,
/// read in one request. A read that fails is logged once per episode and reads as
/// no markers.
fn markers_of_alive(
    relay: &mut Relay,
    regs: &[Registration],
    verdicts: &[(usize, Verdict)],
    reported: &mut Reported,
    log: &mut impl FnMut(String),
) -> relay_core::Result<HashSet<AgentName>> {
    let alive: Vec<&AgentName> = verdicts
        .iter()
        .filter(|(_, v)| *v == Verdict::Alive)
        .map(|(i, _)| &regs[*i].name)
        .collect();
    match relay.panes_replaced(&alive) {
        Ok(found) => {
            reported.marker_read_failing = false;
            Ok(found.found.into_keys().collect())
        }
        Err(e) if e.is_connection() => Err(e),
        Err(e) => {
            if !std::mem::replace(&mut reported.marker_read_failing, true) {
                log(format!("could not read the replaced-pane markers: {e}"));
            }
            Ok(HashSet::new())
        }
    }
}

/// Relinks `reg` to `to`, unless `to`'s place is unknown or another registration
/// of the same session already names `to`; each such lasting refusal is logged
/// once. Returns the event when the registration moved.
fn relink_one(
    relay: &mut Relay,
    layout: &Layout,
    regs: &[Registration],
    reg: &Registration,
    to: &PaneId,
    now_ms: i64,
    reported: &mut Reported,
    log: &mut impl FnMut(String),
) -> relay_core::Result<Option<relay_core::moves::MoveEvent>> {
    let name = &reg.name;
    // A relink records the new place with the new pane; without one the
    // registration would name a pane and an address that disagree.
    let Some(place) = layout.places.get(to) else {
        if reported.trouble.insert(name.clone()) {
            log(format!(
                "{name}: its session is on pane {to}, which herdr gave no workspace or tab \
                 label, so its registration is not moved"
            ));
        }
        return Ok(None);
    };
    let target = PaneRef::new(reg.machine.clone(), to.clone());
    let held = regs.iter().any(|other| {
        other.name != reg.name
            && other.session_id == reg.session_id
            && other.pane_ref().as_ref() == Some(&target)
    });
    if held {
        if reported.trouble.insert(name.clone()) {
            log(format!(
                "{name}: pane {to} is already registered by another lead with the same \
                 session, so its registration is not moved"
            ));
        }
        return Ok(None);
    }
    match relay.relink_pane(reg, to, place, now_ms) {
        Ok(Some(event)) => {
            reported.trouble.remove(name);
            reported.marker_trouble.remove(name);
            log(format!(
                "{name}: its session moved from pane {} to {to}; registration relinked",
                reg.herdr_pane.as_ref().map_or("-", PaneId::as_str)
            ));
            Ok(Some(event))
        }
        Ok(None) => Ok(None),
        Err(e) => {
            once_per_episode(
                &mut reported.marker_trouble,
                name,
                e,
                "could not relink its registration",
                log,
            )?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use herdr_cli::{Output, ProcessRunner};
    use proptest::prelude::*;
    use relay_core::{Place, moves::MoveKind};
    use relay_test_support::Bus;

    use super::*;
    use crate::{
        WatcherState,
        herdr::{AgentGet, Herdr, ProcessError, read_layout},
        refresh_places,
        test_support::{Fault, FaultProxy},
    };

    fn sid(s: &str) -> SessionId {
        s.parse().unwrap()
    }

    fn pane(s: &str) -> PaneId {
        s.parse().unwrap()
    }

    /// Tracing: TC-043-02
    /// ACs: FR-043-AC-2
    #[test]
    fn address_location_requires_a_matching_herdr_session_id() {
        let reg = Registration {
            name: name("lead"),
            machine: hub(),
            kind: "codex".into(),
            session_id: Some(sid("session-current")),
            cwd: None,
            herdr_pane: Some(pane("wB:p1")),
            place: None,
            last_seen_ms: 1,
        };
        let layout = |session| Layout {
            sessions: [(pane("wB:p1"), session)].into_iter().collect(),
            ..Layout::default()
        };
        let with_place = |session| {
            let mut layout = layout(session);
            layout.places.insert(
                pane("wB:p1"),
                relay_core::Place {
                    workspace: "EA".into(),
                    tab: "code".into(),
                    herdr_ids: Some(relay_core::HerdrPlaceIds {
                        workspace: "wB".parse().unwrap(),
                        tab: "wB:t1".parse().unwrap(),
                        pane: "wB:p1".parse().unwrap(),
                        session_id: "session-current".parse().unwrap(),
                        verified_at_ms: 1,
                    }),
                },
            );
            layout
        };
        assert!(pane_has_current_location(
            &reg,
            &with_place(Some(sid("session-current")))
        ));
        assert!(!pane_has_current_location(
            &reg,
            &with_place(Some(sid("session-old")))
        ));
        let mut mismatched_proof = with_place(Some(sid("session-current")));
        mismatched_proof
            .places
            .get_mut(&pane("wB:p1"))
            .unwrap()
            .herdr_ids
            .as_mut()
            .unwrap()
            .session_id = sid("session-old");
        assert!(!pane_has_current_location(&reg, &mismatched_proof));
        assert!(!pane_has_current_location(&reg, &with_place(None)));
        let mut unplaced = reg;
        unplaced.session_id = None;
        assert!(!pane_has_current_location(
            &unplaced,
            &with_place(Some(sid("session-current")))
        ));
    }

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    fn hub() -> MachineName {
        "hub".parse().unwrap()
    }

    fn listed(rows: &[(&str, Option<&str>)]) -> HashMap<PaneId, Option<SessionId>> {
        rows.iter().map(|(p, s)| (pane(p), s.map(sid))).collect()
    }

    // -- the pure decision --------------------------------------------------

    /// Tracing: TC-043-01
    /// ACs: FR-043-AC-1
    #[test]
    fn tc_043_01_a_session_on_another_pane_is_followed_only_when_the_old_pane_is_gone_or_replaced()
    {
        let s = sid("s1");
        let old = pane("w1:p3");
        // Measured on herdr 0.9.1: a cross-workspace move gives `w1:p3` the id `w2:p2`.
        assert_eq!(
            judge(
                &s,
                &old,
                &listed(&[("w2:p2", Some("s1")), ("w1:p1", Some("s0"))])
            ),
            Verdict::Relink(pane("w2:p2"))
        );
        // The old pane now hosts someone else and the session is elsewhere.
        assert_eq!(
            judge(
                &s,
                &old,
                &listed(&[("w1:p3", Some("s9")), ("w2:p2", Some("s1"))])
            ),
            Verdict::Relink(pane("w2:p2"))
        );
        // Still hosted where registered: nothing to do, even if a twin exists.
        assert_eq!(
            judge(
                &s,
                &old,
                &listed(&[("w1:p3", Some("s1")), ("w2:p2", Some("s1"))])
            ),
            Verdict::Alive
        );
    }

    /// Tracing: TC-043-02
    /// ACs: FR-043-AC-2
    #[test]
    fn tc_043_02_anything_uncertain_relinks_nothing() {
        let s = sid("s1");
        let old = pane("w1:p3");
        // The old pane shows an agent with no session yet: it may be the lead.
        assert_eq!(
            judge(&s, &old, &listed(&[("w1:p3", None), ("w2:p2", Some("s1"))])),
            Verdict::Unknown
        );
        // Two panes claim the session.
        assert_eq!(
            judge(
                &s,
                &old,
                &listed(&[("w2:p2", Some("s1")), ("w3:p1", Some("s1"))])
            ),
            Verdict::Ambiguous
        );
        // Neither pane nor session is shown (herdr just restarted).
        assert_eq!(judge(&s, &old, &listed(&[])), Verdict::Unknown);
        // Another session is in the pane and nobody hosts ours: replaced.
        assert_eq!(
            judge(&s, &old, &listed(&[("w1:p3", Some("s9"))])),
            Verdict::Replaced
        );
    }

    fn arbitrary_listing() -> impl Strategy<Value = HashMap<PaneId, Option<SessionId>>> {
        proptest::collection::hash_map(
            (1..4_u8, 1..4_u8).prop_map(|(w, p)| pane(&format!("w{w}:p{p}"))),
            proptest::option::of((1..4_u8).prop_map(|n| sid(&format!("s{n}")))),
            0..9,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Whatever herdr lists, a relink target hosts exactly the registered
        /// session, is the only pane that does, and is not the registered pane.
        ///
        /// Tracing: TC-043-01
        /// ACs: FR-043-AC-1, FR-033-AC-3
        #[test]
        fn tc_043_01_a_relink_target_always_hosts_the_registered_session_alone(
            listing in arbitrary_listing(),
            session in (1..4_u8).prop_map(|n| sid(&format!("s{n}"))),
            registered in (1..4_u8, 1..4_u8).prop_map(|(w, p)| pane(&format!("w{w}:p{p}"))),
        ) {
            let hosts = |s: &SessionId| listing.iter().filter(|(_, v)| v.as_ref() == Some(s)).count();
            match judge(&session, &registered, &listing) {
                Verdict::Relink(to) => {
                    prop_assert_eq!(listing.get(&to), Some(&Some(session.clone())));
                    prop_assert!(to != registered);
                    prop_assert_eq!(listing.iter().filter(|(p, v)| **p != registered && v.as_ref() == Some(&session)).count(), 1);
                    // The registered pane is gone or hosts a *different* session.
                    prop_assert!(matches!(listing.get(&registered), None | Some(Some(_))));
                    prop_assert!(listing.get(&registered) != Some(&Some(session.clone())));
                }
                Verdict::Alive => prop_assert_eq!(listing.get(&registered), Some(&Some(session.clone()))),
                Verdict::Replaced => {
                    prop_assert_eq!(hosts(&session), 0);
                    prop_assert!(matches!(listing.get(&registered), Some(Some(other)) if *other != session));
                }
                Verdict::Ambiguous => prop_assert!(hosts(&session) >= 2),
                Verdict::Unknown => {}
            }
        }
    }

    // -- against a real Redis, replaying a real herdr's output ---------------

    /// One recorded state of a private herdr 0.9.1 server as its three listings
    /// (`agent list`, `workspace list`, `tab list`); see
    /// `spec/reviews/herdr-id-stability.md`.
    macro_rules! snapshot {
        ($name:literal) => {
            [
                include_str!(concat!(
                    "../fixtures/herdr-0.9.1/",
                    $name,
                    ".agent-list.json"
                )),
                include_str!(concat!(
                    "../fixtures/herdr-0.9.1/",
                    $name,
                    ".workspace-list.json"
                )),
                include_str!(concat!("../fixtures/herdr-0.9.1/", $name, ".tab-list.json")),
            ]
        };
    }

    fn recorded(name: &str) -> [&'static str; 3] {
        match name {
            "start" => snapshot!("start"),
            "workspace-renamed" => snapshot!("workspace-renamed"),
            "tab-renamed" => snapshot!("tab-renamed"),
            "pane-moved-same-workspace" => snapshot!("pane-moved-same-workspace"),
            "pane-moved-other-workspace" => snapshot!("pane-moved-other-workspace"),
            "neighbours-closed" => snapshot!("neighbours-closed"),
            "server-restarted" => snapshot!("server-restarted"),
            "path-kind" => snapshot!("path-kind"),
            other => panic!("no recording named {other}"),
        }
    }

    /// Replays a recorded state as the process runner herdr's listings go through.
    struct Replay(&'static str);

    impl ProcessRunner for Replay {
        fn run(
            &self,
            _program: &str,
            args: &[&str],
            _timeout: Duration,
        ) -> Result<Output, ProcessError> {
            let [agents, workspaces, tabs] = recorded(self.0);
            let text = match args {
                ["agent", "list"] => agents,
                ["workspace", "list"] => workspaces,
                ["tab", "list"] => tabs,
                other => panic!("the watcher ran an unexpected herdr command: {other:?}"),
            };
            Ok(Output {
                status_code: 0,
                success: true,
                stdout: text.as_bytes().to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    fn layout_of(snapshot: &'static str) -> Layout {
        read_layout(Replay(snapshot), &hub(), Duration::from_secs(1))
            .expect("a real herdr's output decodes")
            .0
    }

    /// A herdr whose layout is what `layout` says, or a failure when it is `Err`.
    struct FakeHerdr(Result<Layout, ()>);

    impl Herdr for FakeHerdr {
        fn get(&self, _pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(None)
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
        fn layout(&self, _machine: &MachineName) -> Result<Layout, ProcessError> {
            self.0.clone().map_err(|()| ProcessError::Timeout {
                program: "herdr".into(),
                secs: 10,
            })
        }
    }

    fn replayed(snapshot: &'static str) -> FakeHerdr {
        FakeHerdr(Ok(layout_of(snapshot)))
    }

    fn register(
        bus: &mut Bus,
        n: &str,
        machine: &str,
        session: Option<&str>,
        pane: &str,
        place: Option<Place>,
    ) {
        bus.relay
            .register(&Registration {
                name: name(n),
                machine: machine.parse().unwrap(),
                kind: "claude".into(),
                session_id: session.map(sid),
                cwd: None,
                herdr_pane: Some(self::pane(pane)),
                place,
                last_seen_ms: 1_000,
            })
            .unwrap();
    }

    /// The replaced-pane markers of `leads`.
    fn markers_of(
        bus: &mut Bus,
        leads: &[&str],
    ) -> std::collections::BTreeMap<AgentName, PaneReplaced> {
        let names: Vec<AgentName> = leads.iter().map(|n| name(n)).collect();
        let refs: Vec<&AgentName> = names.iter().collect();
        bus.relay.panes_replaced(&refs).unwrap().found
    }

    fn get(bus: &mut Bus, n: &str) -> Registration {
        bus.relay.agent(&name(n)).unwrap().unwrap()
    }

    /// One place-refresh period passes and the watcher reads `herdr`.
    fn refresh(bus: &mut Bus, herdr: &FakeHerdr, state: &mut WatcherState, log: &mut Vec<String>) {
        state.last_place_refresh = None;
        // The gap that limits one lead's rename events has passed (the flapping
        // test uses `refresh_now`).
        state.last_rename_event.clear();
        refresh_now(bus, herdr, state, log);
    }

    /// [`refresh`] without letting the rename gap pass.
    fn refresh_now(
        bus: &mut Bus,
        herdr: &FakeHerdr,
        state: &mut WatcherState,
        log: &mut Vec<String>,
    ) {
        state.last_place_refresh = None;
        let mut snapshot = bus.relay.agents().unwrap().registrations;
        refresh_places(
            &mut bus.relay,
            herdr,
            &mut snapshot,
            &hub(),
            state,
            &mut |line| {
                log.push(line);
            },
        )
        .unwrap();
    }

    /// What the moves stream holds, oldest first, as (kind, lead, from, to).
    fn moves(bus: &mut Bus) -> Vec<(MoveKind, String, Option<String>, Option<String>)> {
        let mut found = bus.relay.recent_moves(100).unwrap().entries;
        found.reverse();
        found
            .into_iter()
            .map(|e| {
                (
                    e.kind,
                    e.lead.to_string(),
                    e.previous_pane.map(|p| p.to_string()),
                    e.pane.map(|p| p.to_string()),
                )
            })
            .collect()
    }

    fn row(
        kind: MoveKind,
        lead: &str,
        from: Option<&str>,
        to: &str,
    ) -> (MoveKind, String, Option<String>, Option<String>) {
        (kind, lead.into(), from.map(str::to_owned), Some(to.into()))
    }

    fn place(workspace: &str, tab: &str, _tabs: u32) -> Place {
        Place {
            workspace: workspace.into(),
            tab: tab.into(),
            herdr_ids: None,
        }
    }

    /// The whole measured sequence, replayed through the watcher's refresh: a
    /// workspace rename, a tab rename, a pane move within its workspace (same id,
    /// new tab), a pane move to another workspace (new id), neighbours closing and
    /// a server restart. One event per rename and per relink, none for anything
    /// else, and the lead keeps its bus name throughout.
    ///
    /// Tracing: TC-043-03
    /// ACs: FR-043-AC-3, FR-034-AC-2, FR-034-AC-3, FR-033-AC-1
    #[test]
    fn tc_043_03_a_measured_herdr_session_is_followed_with_one_event_per_change() {
        let Some(mut bus) = Bus::connect("relink-replay") else {
            return;
        };
        let start = layout_of("start");
        for (n, session, p) in [
            ("lead-a", "sess-A1", "w1:p1"),
            ("lead-b", "sess-A2", "w1:p2"),
            ("lead-c", "sess-A3", "w1:p3"),
            ("lead-d", "sess-B1", "w2:p1"),
        ] {
            let at = start.places.get(&pane(p)).cloned();
            register(&mut bus, n, "hub", Some(session), p, at);
        }
        let mut state = WatcherState::default();
        let mut log = Vec::new();
        let (rename, relink) = (MoveKind::Rename, MoveKind::Relink);
        let mut expect = Vec::new();

        // Workspace `wsA` renamed: the three leads in it are renamed, ids untouched.
        refresh(
            &mut bus,
            &replayed("workspace-renamed"),
            &mut state,
            &mut log,
        );
        expect.extend([
            row(rename, "lead-a", None, "w1:p1"),
            row(rename, "lead-b", None, "w1:p2"),
            row(rename, "lead-c", None, "w1:p3"),
        ]);
        let mut seen = moves(&mut bus);
        seen.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(seen, expect);
        assert!(place_matches(
            &get(&mut bus, "lead-c").place,
            &place("wsA-renamed", "1", 2)
        ));
        assert!(place_matches(
            &get(&mut bus, "lead-d").place,
            &place("wsB", "1", 1)
        ));

        // Tab `tabB` renamed: only its lead.
        refresh(&mut bus, &replayed("tab-renamed"), &mut state, &mut log);
        expect.push(row(rename, "lead-b", None, "w1:p2"));
        assert_eq!(moves(&mut bus).last(), expect.last());
        assert_eq!(moves(&mut bus).len(), expect.len());

        // Ten polls with nothing changed: no event (FR-034-AC-3).
        for _ in 0..10 {
            refresh(&mut bus, &replayed("tab-renamed"), &mut state, &mut log);
        }
        assert_eq!(moves(&mut bus).len(), expect.len());

        // `w1:p3` moved to another tab of its workspace: the pane id is the same,
        // so this is a place change, not a relink.
        refresh(
            &mut bus,
            &replayed("pane-moved-same-workspace"),
            &mut state,
            &mut log,
        );
        expect.push(row(rename, "lead-c", None, "w1:p3"));
        assert_eq!(moves(&mut bus).last(), expect.last());
        assert_eq!(get(&mut bus, "lead-c").herdr_pane, Some(pane("w1:p3")));
        assert!(place_matches(
            &get(&mut bus, "lead-c").place,
            &place("wsA-renamed", "tabB-renamed", 2)
        ));

        // `w1:p3` moved to workspace `wsB`: herdr gave it the id `w2:p2` and kept
        // its session, so the registration follows the session.
        refresh(
            &mut bus,
            &replayed("pane-moved-other-workspace"),
            &mut state,
            &mut log,
        );
        expect.push(row(relink, "lead-c", Some("w1:p3"), "w2:p2"));
        assert_eq!(moves(&mut bus).last(), expect.last());
        let moved = get(&mut bus, "lead-c");
        assert_eq!(moved.herdr_pane, Some(pane("w2:p2")));
        assert!(place_matches(&moved.place, &place("wsB", "1", 1)));
        assert_eq!(moved.session_id, Some(sid("sess-A3")));
        assert!(
            log.iter()
                .any(|l| l.contains("lead-c") && l.contains("relinked"))
        );

        // Neighbours closed (lead-a's tab, and with it its pane and session): the
        // lead remains registered for the prune rule, but its location proof is
        // invalidated because Herdr can no longer verify that pane.
        let before_a = get(&mut bus, "lead-a");
        refresh(
            &mut bus,
            &replayed("neighbours-closed"),
            &mut state,
            &mut log,
        );
        // Only the workspace's tab count changed for lead-b: its address is kept
        // true and no rename is recorded.
        assert!(place_matches(
            &get(&mut bus, "lead-b").place,
            &place("wsA-renamed", "tabB-renamed", 1)
        ));
        let after_a = get(&mut bus, "lead-a");
        assert_eq!(after_a.place, None);
        assert_eq!(after_a.herdr_pane, before_a.herdr_pane);
        assert_eq!(after_a.session_id, before_a.session_id);
        assert_eq!(moves(&mut bus), expect);

        // The server restarted: every pane id survived but no harness has
        // reported a session yet, so old place proofs are invalidated. The leads
        // stay registered and no move is recorded.
        let before: Vec<_> = ["lead-a", "lead-b", "lead-c", "lead-d"]
            .map(|n| get(&mut bus, n))
            .into();
        refresh(
            &mut bus,
            &replayed("server-restarted"),
            &mut state,
            &mut log,
        );
        let after: Vec<_> = ["lead-a", "lead-b", "lead-c", "lead-d"]
            .map(|n| get(&mut bus, n))
            .into();
        for (before, after) in before.iter().zip(&after) {
            assert_eq!(before.name, after.name);
            assert_eq!(before.session_id, after.session_id);
            assert_eq!(before.herdr_pane, after.herdr_pane);
            assert_eq!(after.place, None);
        }
        assert_eq!(moves(&mut bus), expect);
        assert!(
            markers_of(&mut bus, &["lead-a", "lead-b", "lead-c", "lead-d"]).is_empty(),
            "no pane hosted another session at any point"
        );
    }

    fn layout_with(rows: &[(&str, Option<&str>)], places: &[(&str, Place)]) -> Layout {
        let sessions = listed(rows);
        let mut places: HashMap<PaneId, Place> =
            places.iter().map(|(p, at)| (pane(p), at.clone())).collect();
        for (pane, place) in &mut places {
            if let Some(Some(session_id)) = sessions.get(pane) {
                place.herdr_ids = Some(relay_core::HerdrPlaceIds {
                    workspace: "test-workspace".parse().unwrap(),
                    tab: "test-tab".parse().unwrap(),
                    pane: pane.clone(),
                    session_id: session_id.clone(),
                    verified_at_ms: 1,
                });
            }
        }
        Layout {
            places,
            sessions,
            view: None,
        }
    }

    fn place_matches(actual: &Option<Place>, expected: &Place) -> bool {
        actual.as_ref().is_some_and(|actual| {
            actual.workspace == expected.workspace
                && actual.tab == expected.tab
                && actual.herdr_ids.is_some()
        })
    }

    fn run_pass(
        bus: &mut Bus,
        layout: &Layout,
        machine: &str,
        at: i64,
        reported: &mut Reported,
    ) -> Vec<String> {
        let mut snapshot = bus.relay.agents().unwrap().registrations;
        let mut log = Vec::new();
        relink_moved(
            &mut bus.relay,
            layout,
            &mut snapshot,
            &machine.parse().unwrap(),
            at,
            reported,
            &mut |line| log.push(line),
        )
        .unwrap();
        log
    }

    /// Tracing: TC-043-02
    /// ACs: FR-043-AC-2, FR-033-AC-4
    #[test]
    fn tc_043_02_the_guards_hold_at_the_watcher_a_pane_of_another_session_another_machine_no_session()
     {
        let Some(mut bus) = Bus::connect("relink-guards-watcher") else {
            return;
        };
        let here = place("wsA", "1", 1);
        let there = place("wsB", "1", 1);
        // lead-a: session s1, registered on w1:p3, which now hosts s9; s1 is on w2:p2.
        register(
            &mut bus,
            "lead-a",
            "hub",
            Some("s1"),
            "w1:p3",
            Some(here.clone()),
        );
        // lead-t: the same ids on another machine: a different herdr's panes.
        register(
            &mut bus,
            "lead-t",
            "node-a",
            Some("s1"),
            "w1:p3",
            Some(here.clone()),
        );
        // lead-n: no session, pane w1:p4.
        register(&mut bus, "lead-n", "hub", None, "w1:p4", Some(here.clone()));
        let layout = layout_with(
            &[
                ("w1:p3", Some("s9")),
                ("w2:p2", Some("s1")),
                ("w1:p4", Some("s5")),
            ],
            &[
                ("w2:p2", there.clone()),
                ("w1:p3", here.clone()),
                ("w1:p4", here.clone()),
            ],
        );
        let node_a_before = get(&mut bus, "lead-t");
        let none_before = get(&mut bus, "lead-n");
        let mut reported = Reported::default();
        let log = run_pass(&mut bus, &layout, "hub", 5_000, &mut reported);

        assert_eq!(get(&mut bus, "lead-a").herdr_pane, Some(pane("w2:p2")));
        assert_eq!(
            get(&mut bus, "lead-t"),
            node_a_before,
            "another machine's registration is untouched"
        );
        assert_eq!(
            get(&mut bus, "lead-n"),
            none_before,
            "no session id: never relinked"
        );
        assert_eq!(log.len(), 1, "one line for the one relink: {log:?}");
        assert_eq!(
            moves(&mut bus),
            [row(MoveKind::Relink, "lead-a", Some("w1:p3"), "w2:p2")]
        );
    }

    /// Tracing: TC-032-02
    /// ACs: FR-032-AC-2, FR-032-AC-4
    #[test]
    fn tc_032_02_a_pane_hosting_another_session_is_recorded_once_and_ends_when_the_session_is_back()
    {
        let Some(mut bus) = Bus::connect("relink-replaced-watcher") else {
            return;
        };
        let here = place("wsA", "1", 1);
        register(
            &mut bus,
            "lead-a",
            "hub",
            Some("s1"),
            "w1:p3",
            Some(here.clone()),
        );
        let replaced = layout_with(&[("w1:p3", Some("s9"))], &[("w1:p3", place("wsA", "1", 1))]);
        let mut reported = Reported::default();

        let first = run_pass(&mut bus, &replaced, "hub", 5_000, &mut reported);
        let again = run_pass(&mut bus, &replaced, "hub", 9_000, &mut reported);
        assert_eq!(first.len(), 1);
        assert!(again.is_empty(), "the finding is logged once: {again:?}");
        let found = markers_of(&mut bus, &["lead-a"]);
        assert_eq!(
            found[&name("lead-a")].since_ms,
            5_000,
            "since the first finding"
        );
        assert_eq!(found[&name("lead-a")].pane, pane("w1:p3"));
        assert_eq!(found[&name("lead-a")].seen_last_seen_ms, 1_000);
        assert_eq!(
            get(&mut bus, "lead-a").herdr_pane,
            Some(pane("w1:p3")),
            "the registration is kept"
        );
        assert!(moves(&mut bus).is_empty(), "replaced is not a move");

        // The pane's place is another lead's: it is not copied onto the registration
        // as a rename, and no event is recorded.
        let other = layout_with(&[("w1:p3", Some("s9"))], &[("w1:p3", place("wsZ", "9", 4))]);
        let mut state = WatcherState::default();
        refresh(&mut bus, &FakeHerdr(Ok(other)), &mut state, &mut Vec::new());
        assert_eq!(
            get(&mut bus, "lead-a").place,
            None,
            "another session in the pane invalidates its previous location proof"
        );
        assert!(moves(&mut bus).is_empty());

        // The lead's session is in its pane again (a resume): the marker ends.
        let back = layout_with(&[("w1:p3", Some("s1"))], &[("w1:p3", here)]);
        run_pass(&mut bus, &back, "hub", 12_000, &mut reported);
        assert!(markers_of(&mut bus, &["lead-a", "lead-b", "lead-c", "lead-d"]).is_empty());
    }

    /// Tracing: TC-043-02
    /// ACs: FR-043-AC-2
    #[test]
    fn tc_043_02_an_ambiguous_session_a_held_target_and_an_unplaced_target_move_nothing() {
        let Some(mut bus) = Bus::connect("relink-refusals") else {
            return;
        };
        let here = place("wsA", "1", 1);
        register(
            &mut bus,
            "lead-a",
            "hub",
            Some("s1"),
            "w1:p1",
            Some(here.clone()),
        );
        register(
            &mut bus,
            "lead-b",
            "hub",
            Some("s2"),
            "w1:p2",
            Some(here.clone()),
        );
        register(
            &mut bus,
            "lead-c",
            "hub",
            Some("s3"),
            "w1:p4",
            Some(here.clone()),
        );
        // A held target: another registration of the same session names the pane.
        register(
            &mut bus,
            "lead-c2",
            "hub",
            Some("s3"),
            "w2:p3",
            Some(here.clone()),
        );
        let before: Vec<_> = ["lead-a", "lead-b", "lead-c", "lead-c2"]
            .map(|n| get(&mut bus, n))
            .into();
        let layout = layout_with(
            &[
                // lead-a's session on two panes.
                ("w2:p1", Some("s1")),
                ("w2:p2", Some("s1")),
                // lead-b's session on one pane herdr gave no place.
                ("w3:p1", Some("s2")),
                // lead-c's session on the pane lead-c2 holds.
                ("w2:p3", Some("s3")),
            ],
            &[
                ("w2:p1", here.clone()),
                ("w2:p2", here.clone()),
                ("w2:p3", here.clone()),
            ],
        );
        let mut reported = Reported::default();
        let log = run_pass(&mut bus, &layout, "hub", 5_000, &mut reported);
        let after: Vec<_> = ["lead-a", "lead-b", "lead-c", "lead-c2"]
            .map(|n| get(&mut bus, n))
            .into();
        assert_eq!(before, after);
        assert!(moves(&mut bus).is_empty());
        assert_eq!(
            log,
            [
                "lead-a: its session is hosted by more than one pane, so its registration is not moved",
                "lead-b: its session is on pane w3:p1, which herdr gave no workspace or tab label, so its registration is not moved",
                "lead-c: pane w2:p3 is already registered by another lead with the same session, so its registration is not moved",
            ],
            "each lasting refusal is logged once, the missing place included"
        );
        let again = run_pass(&mut bus, &layout, "hub", 9_000, &mut reported);
        assert!(again.is_empty(), "and not again: {again:?}");
    }

    /// Tracing: TC-043-06
    /// ACs: FR-043-AC-6
    #[test]
    fn tc_043_06_a_herdr_that_cannot_answer_changes_nothing_and_logs_one_episode() {
        let Some(mut bus) = Bus::connect("relink-herdr-down") else {
            return;
        };
        let start = layout_of("start");
        let at = start.places.get(&pane("w1:p3")).cloned();
        register(&mut bus, "lead-c", "hub", Some("sess-A3"), "w1:p3", at);
        let before = get(&mut bus, "lead-c");
        let mut state = WatcherState::default();
        let mut log = Vec::new();
        for _ in 0..5 {
            refresh(&mut bus, &FakeHerdr(Err(())), &mut state, &mut log);
        }
        assert_eq!(get(&mut bus, "lead-c"), before);
        assert!(moves(&mut bus).is_empty());
        assert_eq!(
            log.len(),
            1,
            "one line for the episode, not one per refresh: {log:?}"
        );

        // It comes back: the move that happened meanwhile is followed, and the
        // end of the episode is one line.
        refresh(
            &mut bus,
            &replayed("pane-moved-other-workspace"),
            &mut state,
            &mut log,
        );
        assert_eq!(get(&mut bus, "lead-c").herdr_pane, Some(pane("w2:p2")));
        assert!(log.iter().any(|l| l.contains("readable again")));
        // A later failure is a new episode.
        let lines = log.len();
        refresh(&mut bus, &FakeHerdr(Err(())), &mut state, &mut log);
        assert_eq!(log.len(), lines + 1);
    }

    /// Tracing: TC-043-06
    /// ACs: FR-043-AC-6
    #[test]
    fn tc_043_06_a_redis_cut_in_the_middle_of_a_relink_changes_nothing() {
        let Some(mut bus) = Bus::connect("relink-redis-cut") else {
            return;
        };
        let seen = Registration {
            name: name("lead-c"),
            machine: hub(),
            kind: "claude".into(),
            session_id: Some(sid("sess-A3")),
            cwd: None,
            herdr_pane: Some(pane("w1:p3")),
            place: Some(place("wsA", "1", 2)),
            last_seen_ms: 1_000,
        };
        bus.relay.register(&seen).unwrap();
        // The connection dies when the compare-and-set script is sent.
        let proxy = FaultProxy::start(&bus.url, |command| {
            if command.starts_with("EVAL") {
                Fault::Cut
            } else {
                Fault::Forward
            }
        });
        let mut cut = relay_core::Relay::connect(&proxy.url, Duration::from_millis(400))
            .unwrap()
            .with_prefix(bus.prefix.clone());
        let failed = cut
            .relink_pane(&seen, &pane("w2:p2"), &place("wsB", "1", 1), 2_000)
            .unwrap_err();
        assert!(failed.is_connection(), "{failed}");
        // Nothing changed and nothing was recorded: the next pass starts clean.
        assert_eq!(get(&mut bus, "lead-c"), seen);
        assert!(moves(&mut bus).is_empty());
        let moved = bus
            .relay
            .relink_pane(&seen, &pane("w2:p2"), &place("wsB", "1", 1), 3_000)
            .unwrap();
        assert!(moved.is_some());
        assert_eq!(moves(&mut bus).len(), 1);
    }

    /// A herdr for whole ticks: it shows one layout, answers `agent get` only for
    /// the pane that hosts the lead, and records every pane it was asked about and
    /// typed into.
    struct TickHerdr {
        layout: Layout,
        hosts: Option<(PaneId, SessionId)>,
        gets: std::cell::RefCell<Vec<PaneId>>,
        typed: std::cell::RefCell<Vec<PaneId>>,
    }

    impl TickHerdr {
        fn new(snapshot: &'static str, hosts: Option<(&str, &str)>) -> Self {
            Self {
                layout: layout_of(snapshot),
                hosts: hosts.map(|(p, s)| (pane(p), sid(s))),
                gets: std::cell::RefCell::default(),
                typed: std::cell::RefCell::default(),
            }
        }
    }

    impl Herdr for TickHerdr {
        fn get(&self, pane: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            self.gets.borrow_mut().push(pane.clone());
            Ok(self
                .hosts
                .as_ref()
                .filter(|(p, _)| p == pane)
                .map(|(_, s)| AgentGet {
                    status: crate::herdr::AgentStatus::Idle,
                    state_change_seq: 1,
                    session_id: Some(s.clone()),
                }))
        }
        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }
        fn layout(&self, _machine: &MachineName) -> Result<Layout, ProcessError> {
            Ok(self.layout.clone())
        }
    }

    impl crate::deliver::Deliverer for TickHerdr {
        fn deliver(&self, pane: &PaneId, _text: &str) -> Result<(), crate::deliver::DeliverError> {
            self.typed.borrow_mut().push(pane.clone());
            Ok(())
        }
    }

    fn day_ago_ms() -> i64 {
        crate::now_ms() - 25 * 60 * 60 * 1000
    }

    /// The reviewer's sequence: a lead last seen over a day ago (the machine slept),
    /// an empty inbox, its pane moved to another workspace. It is relinked and it
    /// is still registered after the tick: the same tick never prunes the lead it
    /// just found alive (AGE-2033 review, finding 1).
    ///
    /// Tracing: TC-043-03
    /// ACs: FR-043-AC-3
    #[test]
    fn tc_043_03_a_relinked_lead_last_seen_a_day_ago_is_not_pruned_by_the_same_tick() {
        let Some(mut bus) = Bus::connect("relink-then-prune") else {
            return;
        };
        let start = layout_of("start");
        let at = start.places.get(&pane("w1:p3")).cloned();
        register(&mut bus, "lead-c", "hub", Some("sess-A3"), "w1:p3", at);
        let stale = Registration {
            last_seen_ms: day_ago_ms(),
            ..get(&mut bus, "lead-c")
        };
        bus.relay.register(&stale).unwrap();
        // herdr shows the session at its new pane and answers nothing for the old one.
        let herdr = TickHerdr::new("pane-moved-other-workspace", Some(("w2:p2", "sess-A3")));
        let mut state = WatcherState::default();
        let mut log = Vec::new();
        let mut tick = |bus: &mut Bus, state: &mut WatcherState| {
            crate::tick(&mut bus.relay, &herdr, &hub(), state, &mut |l| {
                log.push(l);
            })
            .unwrap();
        };
        tick(&mut bus, &mut state);
        // The cached registry, which the next wakes within its lifetime reuse,
        // carries the new pane, not the one the lead left.
        let cached = state
            .registry
            .as_ref()
            .map(|(_, regs)| regs.clone())
            .unwrap_or_default();
        assert_eq!(
            cached
                .iter()
                .find(|r| r.name == name("lead-c"))
                .and_then(|r| r.herdr_pane.clone()),
            Some(pane("w2:p2"))
        );
        state.wake = crate::Wake::Retry;
        tick(&mut bus, &mut state);
        let after = bus.relay.agent(&name("lead-c")).unwrap();
        assert!(after.is_some(), "the rescued lead is still registered");
        let after = after.unwrap();
        assert_eq!(after.herdr_pane, Some(pane("w2:p2")));
        assert!(
            after.last_seen_ms > day_ago_ms() + 60_000,
            "stamped by the relink"
        );
        assert!(
            !herdr.gets.borrow().contains(&pane("w1:p3")),
            "the pane the lead left was never asked about: {:?}",
            herdr.gets.borrow()
        );
        assert_eq!(moves(&mut bus).len(), 1);
    }

    /// A relinked lead with mail waiting is left out of the rest of the tick: the
    /// pane it left is neither asked about nor typed into, the message stays queued,
    /// and the next tick delivers to the new pane.
    ///
    /// Tracing: TC-043-03
    /// ACs: FR-043-AC-3
    #[test]
    fn tc_043_03_a_relinked_lead_with_mail_is_delivered_at_its_new_pane_on_the_next_tick() {
        let Some(mut bus) = Bus::connect("relink-then-deliver") else {
            return;
        };
        let start = layout_of("start");
        let at = start.places.get(&pane("w1:p3")).cloned();
        register(&mut bus, "lead-c", "hub", Some("sess-A3"), "w1:p3", at);
        bus.relay
            .send(&relay_core::Message {
                from: name("sender"),
                to: relay_core::Target::Agent(name("lead-c")),
                text: "hi".into(),
                sent_at_ms: 1,
            })
            .unwrap();
        let herdr = TickHerdr::new("pane-moved-other-workspace", Some(("w2:p2", "sess-A3")));
        let mut state = WatcherState::default();
        let mut log = Vec::new();
        crate::tick(&mut bus.relay, &herdr, &hub(), &mut state, &mut |l| {
            log.push(l)
        })
        .unwrap();
        assert!(
            herdr.gets.borrow().is_empty(),
            "no herdr call about a pane: {:?}",
            herdr.gets.borrow()
        );
        assert!(herdr.typed.borrow().is_empty());
        assert_eq!(
            bus.relay.inbox_len(&name("lead-c")).unwrap(),
            1,
            "still queued"
        );
        assert_eq!(get(&mut bus, "lead-c").herdr_pane, Some(pane("w2:p2")));

        state.wake = crate::Wake::Sweep;
        crate::tick(&mut bus.relay, &herdr, &hub(), &mut state, &mut |l| {
            log.push(l)
        })
        .unwrap();
        assert_eq!(*herdr.typed.borrow(), [pane("w2:p2")], "{log:?}");
    }

    /// herdr's `agent_session.kind` is `id` or `path`: a path is no session id, so a
    /// healthy lead whose pane reports a path is never marked replaced or relinked
    /// (AGE-2033 review, finding 3).
    ///
    /// Tracing: TC-043-02
    /// ACs: FR-043-AC-2
    #[test]
    fn tc_043_02_a_session_reported_as_a_path_is_no_session_known() {
        let Some(mut bus) = Bus::connect("relink-path-kind") else {
            return;
        };
        let at = place("wsP", "1", 1);
        register(
            &mut bus,
            "lead-a",
            "hub",
            Some("sess-X"),
            "w1:p1",
            Some(at.clone()),
        );
        register(
            &mut bus,
            "lead-b",
            "hub",
            Some("sess-P2"),
            "w1:p2",
            Some(at),
        );
        let layout = layout_of("path-kind");
        assert_eq!(
            layout.sessions[&pane("w1:p1")],
            None,
            "a path reads as no session"
        );
        assert_eq!(layout.sessions[&pane("w1:p2")], Some(sid("sess-P2")));
        let before = get(&mut bus, "lead-a");
        let mut reported = Reported::default();
        let log = run_pass(&mut bus, &layout, "hub", 5_000, &mut reported);
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(get(&mut bus, "lead-a"), before);
        assert!(
            markers_of(&mut bus, &["lead-a", "lead-b"]).is_empty(),
            "nobody is UNDELIVERABLE"
        );
        assert!(moves(&mut bus).is_empty());
    }

    /// A data error on the markers is one line per episode, however many refreshes.
    ///
    /// Tracing: TC-032-02
    /// ACs: FR-032-AC-2
    #[test]
    fn tc_032_02_a_marker_that_cannot_be_written_is_logged_once() {
        let Some(mut bus) = Bus::connect("relink-marker-error") else {
            return;
        };
        register(
            &mut bus,
            "lead-a",
            "hub",
            Some("s1"),
            "w1:p3",
            Some(place("wsA", "1", 1)),
        );
        let () = redis::cmd("LPUSH")
            .arg(bus.key("health:replaced:lead-a"))
            .arg("x")
            .exec(&mut bus.raw)
            .unwrap();
        let replaced = layout_with(&[("w1:p3", Some("s9"))], &[("w1:p3", place("wsA", "1", 1))]);
        let mut reported = Reported::default();
        let mut lines = Vec::new();
        for at in 0..5 {
            lines.extend(run_pass(
                &mut bus,
                &replaced,
                "hub",
                5_000 + at,
                &mut reported,
            ));
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("could not record its replaced pane"),
            "{lines:?}"
        );
    }

    /// A label that flaps is followed but recorded once per gap, with one log line
    /// (review finding 7).
    ///
    /// Tracing: TC-043-03
    /// ACs: FR-043-AC-3
    #[test]
    fn tc_043_03_a_flapping_label_is_one_event_per_gap() {
        let Some(mut bus) = Bus::connect("relink-flap") else {
            return;
        };
        register(
            &mut bus,
            "lead-a",
            "hub",
            Some("s1"),
            "w1:p1",
            Some(place("w0", "1", 1)),
        );
        let mut state = WatcherState::default();
        let mut log = Vec::new();
        for label in ["w1", "w2", "w1", "w2"] {
            let layout = layout_with(&[("w1:p1", Some("s1"))], &[("w1:p1", place(label, "1", 1))]);
            refresh_now(&mut bus, &FakeHerdr(Ok(layout)), &mut state, &mut log);
            assert!(
                place_matches(&get(&mut bus, "lead-a").place, &place(label, "1", 1)),
                "the address follows with current Herdr proof"
            );
        }
        assert_eq!(
            moves(&mut bus).len(),
            1,
            "one event for the first change in the gap"
        );
        assert_eq!(
            log.iter()
                .filter(|l| l.contains("changed again within"))
                .count(),
            1,
            "{log:?}"
        );
    }

    /// Two panes with identical workspace and tab labels: a registration moves
    /// between them only when its session is seen in the other one.
    ///
    /// Tracing: TC-033-03
    /// ACs: FR-033-AC-3
    #[test]
    fn tc_033_03_identical_labels_never_move_a_registration_only_the_session_does() {
        let Some(mut bus) = Bus::connect("relink-identical-labels") else {
            return;
        };
        let same = place("wsA", "1", 1);
        register(
            &mut bus,
            "lead-a",
            "hub",
            Some("s1"),
            "w1:p1",
            Some(same.clone()),
        );
        let both = [("w1:p1", same.clone()), ("w1:p2", same)];
        let mut reported = Reported::default();
        // Another session sits in the other, identically labelled pane: no move.
        let other = layout_with(&[("w1:p1", Some("s1")), ("w1:p2", Some("s2"))], &both);
        run_pass(&mut bus, &other, "hub", 1, &mut reported);
        assert_eq!(get(&mut bus, "lead-a").herdr_pane, Some(pane("w1:p1")));
        // Both panes hold other sessions and its own is nowhere: no move.
        let nowhere = layout_with(&[("w1:p1", Some("s3")), ("w1:p2", Some("s2"))], &both);
        run_pass(&mut bus, &nowhere, "hub", 2, &mut reported);
        assert_eq!(get(&mut bus, "lead-a").herdr_pane, Some(pane("w1:p1")));
        // Its session appears in the other pane: it follows the session.
        let moved = layout_with(&[("w1:p1", Some("s3")), ("w1:p2", Some("s1"))], &both);
        run_pass(&mut bus, &moved, "hub", 3, &mut reported);
        assert_eq!(get(&mut bus, "lead-a").herdr_pane, Some(pane("w1:p2")));
    }
}
