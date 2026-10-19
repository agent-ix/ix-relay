//! Mail stranded in the inbox of a registration whose session left its pane
//! (FR-053).
//!
//! Once per place-refresh period, from the `agent list` that refresh already
//! read, [`pass`] looks at each local registration with a pane, a session id and
//! queued mail whose pane no longer hosts its session:
//!
//! - exactly one other registration of the machine has its address, and herdr
//!   verifies that registration's session in its pane: the mail moves to that
//!   registration ([`Relay::forward_stranded`]), marked as sent to a prior session;
//! - none does and the registration is old enough for the pruning rule: the mail
//!   moves to the dead-letter list and the registration is pruned
//!   ([`Relay::dead_letter_stranded`]), so that mail never waits for a session
//!   that never registers;
//! - anything else (more than one registration has the address, the holder's
//!   session is not verified yet, the registration is too young): nothing moves.
//!
//! A pane that hosts its session, an answer from herdr that lists nothing or names
//! no session for the pane, a registration without a session or a pane, and every
//! registration of another machine are never touched. Both moves are
//! compare-and-sets against the registrations the pass read, so a lead pruned or
//! registered again meanwhile stays as it is.

use std::collections::{HashMap, HashSet};

use relay_core::{Address, AgentName, Buried, MachineName, Registration, Relay};

use crate::{
    herdr::Layout,
    old_enough_to_prune,
    relink::{Verdict, judge, pane_has_current_location},
};

/// What the pass remembers between place refreshes.
#[derive(Debug, Default)]
pub struct Stranded {
    /// Where each local lead last verifiably sat. A registration whose session left
    /// its pane loses its recorded place in the same refresh that finds it gone, so
    /// this is the only record of the address its mail was written to.
    addresses: HashMap<AgentName, Address>,
    /// Leads whose last move failed for a lasting reason, reported once.
    trouble: HashSet<AgentName>,
}

/// How a registration's session left its pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Left {
    /// The pane hosts another session.
    Replaced,
    /// herdr lists no agent in the pane and no pane hosting the session.
    Gone,
}

/// Whether, and how, the listing shows `reg`'s session to have left its pane. `None`
/// for a session that is home and for everything herdr's answer leaves unknown.
fn left(reg: &Registration, layout: &Layout) -> Option<Left> {
    let (session, pane) = (reg.session_id.as_ref()?, reg.herdr_pane.as_ref()?);
    // herdr listed no agent at all (it is restarting): everything is unknown.
    if layout.sessions.is_empty() {
        return None;
    }
    match judge(session, pane, &layout.sessions) {
        Verdict::Replaced => Some(Left::Replaced),
        Verdict::Unknown
            if !layout.sessions.contains_key(pane)
                && !layout
                    .sessions
                    .values()
                    .any(|s| s.as_ref() == Some(session)) =>
        {
            Some(Left::Gone)
        }
        Verdict::Alive | Verdict::Relink(_) | Verdict::Ambiguous | Verdict::Unknown => None,
    }
}

/// What the registrations of `machine` other than `stranded` say about who holds
/// `address` now.
enum Holder<'a> {
    /// No registration has the address.
    None,
    /// Exactly one does, and herdr verifies its session in its pane.
    Verified(&'a Registration),
    /// One that herdr does not verify (yet), or more than one: not resolved by guessing.
    Unresolved,
}

fn holder<'a>(
    stranded: &Registration,
    address: &Address,
    regs: &'a [Registration],
    layout: &Layout,
) -> Holder<'a> {
    let mut sharing = regs.iter().filter(|r| {
        r.machine == stranded.machine
            && r.name != stranded.name
            && r.full_address().is_some_and(|a| a == *address)
    });
    match (sharing.next(), sharing.next()) {
        (None, _) => Holder::None,
        (Some(one), None) if pane_has_current_location(one, layout) => Holder::Verified(one),
        _ => Holder::Unresolved,
    }
}

impl Stranded {
    /// Remembers the address of each lead on `machine` that has one now: called by
    /// the refresh before it drops the places of registrations whose session left.
    pub fn observe(&mut self, regs: &[Registration], machine: &MachineName) {
        for reg in regs.iter().filter(|r| r.machine == *machine) {
            if let Some(address) = reg.full_address() {
                self.addresses.insert(reg.name.clone(), address);
            }
        }
        self.addresses
            .retain(|name, _| regs.iter().any(|r| r.name == *name));
        self.trouble
            .retain(|name| regs.iter().any(|r| r.name == *name));
    }
}

/// Runs the pass over the registrations of `machine` in `regs`, removing from
/// `regs` the ones it pruned (so the rest of the tick does not act on them).
///
/// A failure of one lead's move is logged once per episode; only a
/// connection-level Redis error is returned, so the tick ends and Redis is dialled
/// again.
///
/// # Errors
///
/// A connection-level Redis error.
pub fn pass(
    state: &mut Stranded,
    relay: &mut Relay,
    layout: &Layout,
    regs: &mut Vec<Registration>,
    machine: &MachineName,
    log: &mut impl FnMut(String),
) -> relay_core::Result<()> {
    let candidates: Vec<(Registration, Left)> = regs
        .iter()
        .filter(|r| r.machine == *machine)
        .filter_map(|r| Some((r.clone(), left(r, layout)?)))
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }
    let names: Vec<&AgentName> = candidates.iter().map(|(r, _)| &r.name).collect();
    let lens = relay.inbox_lens(&names)?;
    let mut pruned = Vec::new();
    for ((reg, how), len) in candidates.iter().zip(lens) {
        // An inbox that cannot be read is not an empty one: left for the next refresh.
        if !matches!(len, Ok(n) if n > 0) {
            continue;
        }
        let address = state.addresses.get(&reg.name);
        let verdict = match address.map(|a| holder(reg, a, regs, layout)) {
            Some(Holder::Verified(to)) => Move::Forward(to),
            Some(Holder::Unresolved) => continue,
            // Nobody has the address, or it is no longer known: the mail waits only
            // as long as the pruning rule makes a registration wait.
            Some(Holder::None) | None if old_enough_to_prune(reg.last_seen_ms) => Move::Bury,
            Some(Holder::None) | None => continue,
        };
        let done = match verdict {
            Move::Forward(to) => forward(relay, reg, to, *how, log),
            Move::Bury => bury(relay, reg, *how, log),
        };
        match done {
            Ok(Done::Pruned) => pruned.push(reg.name.clone()),
            Ok(Done::Kept) => {
                state.trouble.remove(&reg.name);
            }
            Err(e) if e.is_connection() => return Err(e),
            Err(e) => {
                if state.trouble.insert(reg.name.clone()) {
                    log(format!(
                        "{}: could not move its stranded mail: {e}",
                        reg.name
                    ));
                }
            }
        }
    }
    regs.retain(|r| !pruned.contains(&r.name));
    Ok(())
}

enum Move<'a> {
    Forward(&'a Registration),
    Bury,
}

enum Done {
    /// The registration is gone from the bus.
    Pruned,
    /// The registration stays (it was forwarded from, or nothing changed).
    Kept,
}

fn forward(
    relay: &mut Relay,
    from: &Registration,
    to: &Registration,
    how: Left,
    log: &mut impl FnMut(String),
) -> relay_core::Result<Done> {
    if let Some(moved) = relay.forward_stranded(from, to)?
        && moved > 0
    {
        log(format!(
            "{}: its session {} its pane; forwarded {moved} queued message(s) to {}, which \
             now holds the address",
            from.name,
            match how {
                Left::Replaced => "was replaced in",
                Left::Gone => "is gone from",
            },
            to.name
        ));
    }
    Ok(Done::Kept)
}

fn bury(
    relay: &mut Relay,
    reg: &Registration,
    how: Left,
    log: &mut impl FnMut(String),
) -> relay_core::Result<Done> {
    match relay.dead_letter_stranded(reg)? {
        Buried::Done(moved) => {
            if moved > 0 {
                log(format!(
                    "{}: its session {} its pane and no registration holds its address; \
                     dead-lettered {moved} queued message(s), reason recipient-pruned",
                    reg.name,
                    match how {
                        Left::Replaced => "was replaced in",
                        Left::Gone => "is gone from",
                    },
                ));
            }
            log(format!("{}: stranded registration pruned", reg.name));
            Ok(Done::Pruned)
        }
        Buried::Changed | Buried::InFlight => Ok(Done::Kept),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, num::NonZeroUsize, time::Duration};

    use redis::Commands;
    use relay_core::{
        Claim, HerdrPlaceIds, Message, Now, PaneId, Place, Popper, SessionId, Target,
    };
    use relay_test_support::Bus;

    use super::*;
    use crate::{
        WatcherState,
        deliver::{DeliverError, Deliverer},
        herdr::{AgentGet, AgentStatus, Herdr, ProcessError},
        now_ms,
    };

    const HOUR_MS: i64 = 60 * 60 * 1000;

    fn name(s: &str) -> AgentName {
        s.parse().unwrap()
    }

    fn hub() -> MachineName {
        "hub".parse().unwrap()
    }

    fn sid(s: &str) -> SessionId {
        s.parse().unwrap()
    }

    fn pane(s: &str) -> PaneId {
        s.parse().unwrap()
    }

    /// A registration on `machine`, in `pane` at `workspace:tab`, whose place is
    /// verified for its own session.
    fn reg_on(
        machine: &str,
        n: &str,
        session: &str,
        pane_id: &str,
        tab: &str,
        age_ms: i64,
    ) -> Registration {
        Registration {
            name: name(n),
            machine: machine.parse().unwrap(),
            kind: "claude".into(),
            session_id: Some(sid(session)),
            cwd: None,
            herdr_pane: Some(pane(pane_id)),
            place: Some(place_of(pane_id, session, tab)),
            last_seen_ms: now_ms() - age_ms,
        }
    }

    fn reg(n: &str, session: &str, pane_id: &str, tab: &str, age_ms: i64) -> Registration {
        reg_on("hub", n, session, pane_id, tab, age_ms)
    }

    fn place_of(pane_id: &str, session: &str, tab: &str) -> Place {
        Place {
            workspace: "EA".into(),
            tab: tab.into(),
            herdr_ids: Some(HerdrPlaceIds {
                workspace: "w1".parse().unwrap(),
                tab: "w1:t1".parse().unwrap(),
                pane: pane(pane_id),
                session_id: sid(session),
                verified_at_ms: 1,
            }),
        }
    }

    /// What herdr lists: each `(pane, session, tab)`, with `None` for an agent whose
    /// harness has not reported a session.
    fn layout(rows: &[(&str, Option<&str>, &str)]) -> Layout {
        let mut layout = Layout::default();
        for (p, session, tab) in rows {
            layout.sessions.insert(pane(p), session.map(sid));
            if let Some(session) = session {
                layout.places.insert(pane(p), place_of(p, session, tab));
            }
        }
        layout
    }

    fn put(bus: &mut Bus, regs: &[&Registration]) {
        for r in regs {
            bus.relay.register(r).unwrap();
        }
    }

    fn send(bus: &mut Bus, to: &str, texts: &[&str]) {
        for text in texts {
            bus.relay
                .send(&Message {
                    from: name("hub:EA:sender"),
                    to: Target::Agent(name(to)),
                    text: (*text).into(),
                    sent_at_ms: 1_700_000_000_000,
                    forwarded: false,
                })
                .unwrap();
        }
    }

    fn queued(bus: &mut Bus, lead: &str) -> Vec<String> {
        let key = bus.key(&format!("inbox:{lead}"));
        let entries: Vec<String> = bus.raw.lrange(key, 0, -1).unwrap();
        entries
            .iter()
            .map(|e| serde_json::from_str::<Message>(e).unwrap().text)
            .collect()
    }

    fn dead(bus: &mut Bus, lead: &str) -> Vec<String> {
        let key = bus.key(&format!("deadletter:{lead}"));
        bus.raw.lrange(key, 0, -1).unwrap()
    }

    fn stored(bus: &mut Bus, lead: &str) -> Vec<String> {
        let key = bus.key(&format!("inbox:{lead}"));
        bus.raw.lrange(key, 0, -1).unwrap()
    }

    fn registered(bus: &mut Bus, lead: &str) -> bool {
        bus.relay.agent(&name(lead)).unwrap().is_some()
    }

    /// One pass as the refresh runs it: the addresses are remembered from the
    /// registrations as they were while their sessions were home (`before`), then
    /// the pass sees `regs` and `layout`. Returns the log.
    fn pass_after(
        bus: &mut Bus,
        before: &[&Registration],
        regs: &mut Vec<Registration>,
        layout: &Layout,
    ) -> Vec<String> {
        let mut state = Stranded::default();
        state.observe(&before.iter().copied().cloned().collect::<Vec<_>>(), &hub());
        let mut log = Vec::new();
        pass(&mut state, &mut bus.relay, layout, regs, &hub(), &mut |l| {
            log.push(l);
        })
        .unwrap();
        log
    }

    /// The replaced registration `old` (session `s1`, pane `w1:p1`), the one that
    /// holds its address now `new` (session `s2`, the same pane and tab), and what
    /// herdr lists: `s2` in that pane.
    fn replaced_world() -> (Registration, Registration, Layout) {
        (
            reg("old-lead", "s1", "w1:p1", "code", HOUR_MS),
            reg("new-lead", "s2", "w1:p1", "code", 0),
            layout(&[("w1:p1", Some("s2"), "code")]),
        )
    }

    /// Tracing: TC-053-01
    /// ACs: FR-053-AC-1
    #[test]
    fn tc_053_01_a_pass_hands_the_old_inbox_to_the_one_session_holding_the_address() {
        let Some(mut bus) = Bus::connect("stranded-pass-1") else {
            return;
        };
        let (old, new, layout) = replaced_world();
        put(&mut bus, &[&old, &new]);
        send(&mut bus, "old-lead", &["one", "two", "three"]);
        send(&mut bus, "new-lead", &["own"]);

        // The refresh dropped the old place before the pass; the address was remembered.
        let mut dropped = old.clone();
        dropped.place = None;
        put(&mut bus, &[&dropped]);
        let mut regs = vec![dropped, new.clone()];
        let log = pass_after(&mut bus, &[&old, &new], &mut regs, &layout);

        assert!(queued(&mut bus, "old-lead").is_empty());
        assert_eq!(
            queued(&mut bus, "new-lead"),
            ["one", "two", "three", "own"],
            "in order, before the new session's own mail"
        );
        assert!(registered(&mut bus, "old-lead"), "left in place");
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(
            log[0].contains("old-lead")
                && log[0].contains("new-lead")
                && log[0].contains("forwarded 3"),
            "{log:?}"
        );
        // A pass that does nothing logs nothing.
        let again = pass_after(&mut bus, &[&old, &new], &mut regs, &layout);
        assert!(again.is_empty(), "{again:?}");
    }

    /// Tracing: TC-053-04
    /// ACs: FR-053-AC-4
    #[test]
    fn tc_053_04_a_pane_that_hosts_its_session_never_loses_mail_whatever_the_age() {
        let Some(mut bus) = Bus::connect("stranded-pass-4") else {
            return;
        };
        let (old, new, _) = replaced_world();
        put(&mut bus, &[&old, &new]);
        send(&mut bus, "old-lead", &["one", "two"]);
        // Years old, its pane hosts its session (and so does another pane).
        let mut ancient = old.clone();
        ancient.last_seen_ms = 1;
        let alive = layout(&[("w1:p1", Some("s1"), "code"), ("w1:p2", Some("s2"), "code")]);
        // Anything herdr could answer that is not positive: nothing listed, no
        // session named for the pane, a different pane elsewhere.
        let unknown = [
            Layout::default(),
            layout(&[("w1:p1", None, "code")]),
            layout(&[("w1:p1", None, "code"), ("w1:p5", Some("s1"), "code")]),
        ];
        for listed in std::iter::once(&alive).chain(&unknown) {
            let mut regs = vec![ancient.clone(), new.clone()];
            let log = pass_after(&mut bus, &[&ancient, &new], &mut regs, listed);
            assert!(log.is_empty(), "{log:?}");
            assert_eq!(regs.len(), 2);
            assert_eq!(queued(&mut bus, "old-lead"), ["one", "two"]);
            assert!(queued(&mut bus, "new-lead").is_empty());
            assert!(dead(&mut bus, "old-lead").is_empty());
            assert!(registered(&mut bus, "old-lead"));
        }
    }

    /// Tracing: TC-053-05
    /// ACs: FR-053-AC-5
    #[test]
    fn tc_053_05_an_address_held_by_none_or_by_two_forwards_nothing() {
        let Some(mut bus) = Bus::connect("stranded-pass-5") else {
            return;
        };
        let (old, new, listed) = replaced_world();
        // Held by none (the new occupant never registered): the mail waits.
        put(&mut bus, &[&old]);
        send(&mut bus, "old-lead", &["one"]);
        let mut regs = vec![old.clone()];
        let log = pass_after(&mut bus, &[&old], &mut regs, &listed);
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(queued(&mut bus, "old-lead"), ["one"]);

        // Held by two, one of which is its verified session: still ambiguous, and even
        // when the registration is old enough to be pruned, the mail waits.
        let twin = reg("twin-lead", "s3", "w1:p2", "code", 0);
        put(&mut bus, &[&new, &twin]);
        let mut ancient = old.clone();
        ancient.last_seen_ms = now_ms() - 30 * HOUR_MS;
        let mut regs = vec![ancient.clone(), new.clone(), twin.clone()];
        let log = pass_after(&mut bus, &[&ancient, &new, &twin], &mut regs, &listed);
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(queued(&mut bus, "old-lead"), ["one"]);
        assert!(
            queued(&mut bus, "new-lead").is_empty() && queued(&mut bus, "twin-lead").is_empty()
        );
        assert!(registered(&mut bus, "old-lead"));

        // Held by one whose session herdr does not verify in its pane: not forwarded to.
        let wrong = layout(&[("w1:p1", Some("s9"), "code")]);
        let mut regs = vec![old.clone(), new.clone()];
        bus.relay
            .prune(&name("twin-lead"), twin.last_seen_ms)
            .unwrap();
        let log = pass_after(&mut bus, &[&old, &new], &mut regs, &wrong);
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(queued(&mut bus, "old-lead"), ["one"]);
    }

    /// Tracing: TC-053-05
    /// ACs: FR-053-AC-5
    ///
    /// A herdr that cannot be asked or answers in a shape that cannot be read gives
    /// the refresh no layout: it moves, forwards and prunes nothing.
    #[test]
    fn tc_053_05_a_herdr_that_fails_moves_nothing() {
        let Some(mut bus) = Bus::connect("stranded-herdr-fails") else {
            return;
        };
        let (old, new, _) = replaced_world();
        put(&mut bus, &[&old, &new]);
        send(&mut bus, "old-lead", &["one"]);
        let mut ancient = old.clone();
        ancient.last_seen_ms = now_ms() - 30 * HOUR_MS;
        put(&mut bus, &[&ancient]);
        let herdr = ScriptedHerdr::failing();
        let mut state = WatcherState::default();
        let mut regs = vec![ancient, new];
        let mut log = |_: String| {};
        crate::refresh_places(
            &mut bus.relay,
            &herdr,
            &mut regs,
            &hub(),
            &mut state,
            &mut log,
        )
        .unwrap();
        assert_eq!(queued(&mut bus, "old-lead"), ["one"]);
        assert!(registered(&mut bus, "old-lead"));
        assert_eq!(regs.len(), 2);
    }

    /// Tracing: TC-053-07
    /// ACs: FR-053-AC-7
    #[test]
    fn tc_053_07_a_registration_changed_since_the_pass_read_it_is_left_alone() {
        let Some(mut bus) = Bus::connect("stranded-pass-7") else {
            return;
        };
        let (old, new, listed) = replaced_world();
        put(&mut bus, &[&old, &new]);
        send(&mut bus, "old-lead", &["one"]);
        let mut regs = vec![old.clone(), new.clone()];
        // Both registered again between the read and the pass.
        let mut again = old.clone();
        again.last_seen_ms += 1;
        put(&mut bus, &[&again]);
        let log = pass_after(&mut bus, &[&old, &new], &mut regs, &listed);
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(queued(&mut bus, "old-lead"), ["one"]);

        // And pruned between the read and the pass.
        bus.relay
            .prune(&name("old-lead"), again.last_seen_ms)
            .unwrap();
        let log = pass_after(&mut bus, &[&old, &new], &mut regs, &listed);
        assert!(log.is_empty(), "{log:?}");
        assert!(queued(&mut bus, "new-lead").is_empty());
    }

    /// Tracing: TC-053-09
    /// ACs: FR-053-AC-9
    #[test]
    fn tc_053_09_another_machines_registration_is_never_touched() {
        let Some(mut bus) = Bus::connect("stranded-pass-9") else {
            return;
        };
        // The same pane id and session on `elsewhere`, in every state a pass acts on.
        let old = reg_on("elsewhere", "far-lead", "s1", "w1:p1", "code", 30 * HOUR_MS);
        let new = reg_on("elsewhere", "far-new", "s2", "w1:p1", "code", 0);
        put(&mut bus, &[&old, &new]);
        send(&mut bus, "far-lead", &["one"]);
        for listed in [
            layout(&[("w1:p1", Some("s2"), "code")]),
            layout(&[("w1:p9", Some("s7"), "code")]),
        ] {
            let mut regs = vec![old.clone(), new.clone()];
            let mut state = Stranded::default();
            state.observe(&regs, &"elsewhere".parse().unwrap());
            let mut log = Vec::new();
            pass(
                &mut state,
                &mut bus.relay,
                &listed,
                &mut regs,
                &hub(),
                &mut |l| {
                    log.push(l);
                },
            )
            .unwrap();
            assert!(log.is_empty(), "{log:?}");
            assert_eq!(queued(&mut bus, "far-lead"), ["one"]);
            assert!(dead(&mut bus, "far-lead").is_empty());
            assert!(registered(&mut bus, "far-lead"));
            assert_eq!(regs.len(), 2);
        }
    }

    /// Tracing: TC-053-10
    /// ACs: FR-053-AC-10
    #[test]
    fn tc_053_10_a_gone_registration_old_enough_is_dead_lettered_then_pruned() {
        let Some(mut bus) = Bus::connect("stranded-pass-10") else {
            return;
        };
        let nothing_there = layout(&[("w1:p7", Some("s7"), "other")]);
        let old = reg("old-lead", "s1", "w1:p1", "code", 25 * HOUR_MS);
        put(&mut bus, &[&old]);
        send(&mut bus, "old-lead", &["one", "t\"w\\o"]);
        let before = stored(&mut bus, "old-lead");

        // 23 hours: nothing moves.
        let young = reg("young-lead", "s4", "w1:p4", "tab4", 23 * HOUR_MS);
        put(&mut bus, &[&young]);
        send(&mut bus, "young-lead", &["y"]);
        let mut regs = vec![old.clone(), young.clone()];
        let log = pass_after(&mut bus, &[&old, &young], &mut regs, &nothing_there);
        assert_eq!(
            regs.len(),
            1,
            "only the pruned one left the snapshot: {log:?}"
        );
        assert_eq!(queued(&mut bus, "young-lead"), ["y"]);
        assert!(registered(&mut bus, "young-lead"));

        // 25 hours: both messages are dead letters byte for byte, the registration pruned.
        assert_eq!(dead(&mut bus, "old-lead"), before);
        assert!(stored(&mut bus, "old-lead").is_empty());
        assert!(!registered(&mut bus, "old-lead"));
        assert!(
            log.iter()
                .any(|l| l.contains("dead-lettered 2") && l.contains("recipient-pruned"))
        );
        assert!(log.iter().any(|l| l.contains("pruned")), "{log:?}");
    }

    /// Tracing: TC-053-10
    /// ACs: FR-053-AC-10
    #[test]
    fn tc_053_10_when_dead_lettering_fails_nothing_is_pruned_and_the_mail_stays() {
        let Some(mut bus) = Bus::connect("stranded-pass-10-fails") else {
            return;
        };
        let old = reg("old-lead", "s1", "w1:p1", "code", 25 * HOUR_MS);
        put(&mut bus, &[&old]);
        send(&mut bus, "old-lead", &["one", "two"]);
        let () = bus.raw.set(bus.key("deadletter:old-lead"), "oops").unwrap();
        let nothing_there = layout(&[("w1:p7", Some("s7"), "other")]);
        let mut regs = vec![old.clone()];
        let log = pass_after(&mut bus, &[&old], &mut regs, &nothing_there);
        assert_eq!(queued(&mut bus, "old-lead"), ["one", "two"]);
        assert!(registered(&mut bus, "old-lead"));
        assert_eq!(regs.len(), 1);
        assert_eq!(log.len(), 1, "one line for the failure: {log:?}");
    }

    /// Tracing: TC-053-11
    /// ACs: FR-053-AC-11
    #[test]
    fn tc_053_11_a_leased_batch_waits_for_its_lease_and_the_next_pass_moves_it() {
        let Some(mut bus) = Bus::connect("stranded-pass-11") else {
            return;
        };
        let (old, new, listed) = replaced_world();
        put(&mut bus, &[&old, &new]);
        send(&mut bus, "old-lead", &["one", "two"]);
        let claim = Claim::new(Popper::Watcher)
            .with_lease(Duration::from_secs(60))
            .at(Now::AtMs(1_000));
        let popped = bus
            .relay
            .pop_inbox_budget(
                &name("old-lead"),
                NonZeroUsize::MIN.saturating_add(1),
                usize::MAX,
                claim,
            )
            .unwrap();
        assert_eq!(popped.messages.len(), 2);

        let mut regs = vec![old.clone(), new.clone()];
        let log = pass_after(&mut bus, &[&old, &new], &mut regs, &listed);
        assert!(
            log.is_empty(),
            "nothing is queued, so nothing moves: {log:?}"
        );
        assert!(queued(&mut bus, "new-lead").is_empty());
        assert_eq!(bus.relay.inflight_len(&name("old-lead")).unwrap(), 2);

        bus.relay
            .reap_expired(&name("old-lead"), Now::AtMs(1_000 + 61_000))
            .unwrap();
        let log = pass_after(&mut bus, &[&old, &new], &mut regs, &listed);
        assert_eq!(log.len(), 1, "{log:?}");
        assert_eq!(queued(&mut bus, "new-lead"), ["one", "two"]);
    }

    /// Tracing: TC-053-13
    /// ACs: FR-053-AC-13
    #[test]
    fn tc_053_13_a_replaced_pane_whose_new_occupant_never_registers_is_dead_lettered_at_the_prune_age()
     {
        let Some(mut bus) = Bus::connect("stranded-pass-13") else {
            return;
        };
        // The pane hosts `s9`, which is no registration.
        let listed = layout(&[("w1:p1", Some("s9"), "code")]);
        let at_25h = reg("old-lead", "s1", "w1:p1", "code", 25 * HOUR_MS);
        let at_23h = reg("young-lead", "s2", "w1:p2", "tab2", 23 * HOUR_MS);
        let listed = {
            let mut l = listed;
            l.sessions.insert(pane("w1:p2"), Some(sid("s8")));
            l
        };
        put(&mut bus, &[&at_25h, &at_23h]);
        send(&mut bus, "old-lead", &["one", "two"]);
        send(&mut bus, "young-lead", &["y"]);
        let before = stored(&mut bus, "old-lead");

        let mut regs = vec![at_25h.clone(), at_23h.clone()];
        let log = pass_after(&mut bus, &[&at_25h, &at_23h], &mut regs, &listed);

        assert_eq!(dead(&mut bus, "old-lead"), before);
        assert!(!registered(&mut bus, "old-lead"));
        assert_eq!(
            queued(&mut bus, "young-lead"),
            ["y"],
            "23 hours is too young"
        );
        assert!(registered(&mut bus, "young-lead"));
        assert!(
            log.iter().any(|l| l.contains("recipient-pruned")),
            "{log:?}"
        );
    }

    /// A herdr that lists `layout` (or fails), says each listed agent is idle, and
    /// records what is typed into it.
    struct ScriptedHerdr {
        layout: Option<Layout>,
        prompts: RefCell<Vec<String>>,
    }

    impl ScriptedHerdr {
        fn listing(layout: Layout) -> Self {
            Self {
                layout: Some(layout),
                prompts: RefCell::default(),
            }
        }

        fn failing() -> Self {
            Self {
                layout: None,
                prompts: RefCell::default(),
            }
        }
    }

    impl Herdr for ScriptedHerdr {
        fn get(&self, p: &PaneId) -> Result<Option<AgentGet>, ProcessError> {
            Ok(self
                .layout
                .as_ref()
                .and_then(|l| l.sessions.get(p))
                .map(|session| AgentGet {
                    status: AgentStatus::Idle,
                    state_change_seq: 1,
                    session_id: session.clone(),
                }))
        }

        fn notify(&self, _title: &str, _body: &str) -> Result<(), ProcessError> {
            Ok(())
        }

        fn layout(&self, _machine: &MachineName) -> Result<Layout, ProcessError> {
            self.layout.clone().ok_or(ProcessError::Timeout {
                program: "herdr".into(),
                secs: 10,
            })
        }
    }

    impl Deliverer for ScriptedHerdr {
        fn deliver(&self, _pane: &PaneId, text: &str) -> Result<(), DeliverError> {
            self.prompts.borrow_mut().push(text.to_owned());
            Ok(())
        }
    }

    /// Tracing: TC-053-02
    /// ACs: FR-053-AC-2
    ///
    /// A whole tick, as the watcher runs it: the refresh moves the old session's
    /// mail, and the same tick types it into the new session's pane with the notice on
    /// each message's signed line, naming the original sender and send time. The
    /// socket delivers the same rendered batch (`PoppedBatch::rendered`).
    #[test]
    fn tc_053_02_the_watcher_prompt_carries_the_notice() {
        let Some(mut bus) = Bus::connect("stranded-tick") else {
            return;
        };
        let (old, new, listed) = replaced_world();
        put(&mut bus, &[&old, &new]);
        send(&mut bus, "old-lead", &["one", "two\nsecond line"]);
        let herdr = ScriptedHerdr::listing(listed);
        let mut state = WatcherState::default();
        // The refresh remembers where the old session sat while it was home.
        state.stranded.observe(std::slice::from_ref(&old), &hub());
        let mut dropped = old.clone();
        dropped.place = None;
        bus.relay.register(&dropped).unwrap();

        crate::tick(&mut bus.relay, &herdr, &hub(), &mut state, &mut |_| {}).unwrap();

        let prompts = herdr.prompts.borrow();
        assert_eq!(prompts.len(), 1, "{prompts:?}");
        let notice = "(sent 2023-11-14T22:13:20Z to a prior session of this address)";
        assert_eq!(
            prompts[0],
            format!(
                "[ix-relay] from hub:EA:sender {notice}: one\n\
                 [ix-relay] from hub:EA:sender {notice}: two\n    second line"
            )
        );
    }
}
