//! What the watcher says about itself, and what it keeps true about the leads it
//! watches, apart from delivering mail (AGE-2033).
//!
//! - The heartbeat (FR-028): after every good tick a typed record is written to
//!   `<prefix>health:watcher:<machine>` with a short expiry, so a missing key
//!   means no watcher has ticked on this machine recently.
//! - The herdr check (FR-036): [`crate::herdr_gate`], run before a tick.
//! - The registration refresh (FR-042): [`crate::last_seen`], called per lead.
//!
//! [`Liveness`] is the one value [`crate::WatcherState`] carries for all three, so
//! the tick loop has a hook per concern and none of the logic.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use relay_core::{
    AgentName, MachineName, Relay,
    health::{HeartbeatError, HeartbeatErrorKind, MAX_PROBLEM_CHARS, WatcherHeartbeat},
};
use relay_ops::herdr_check::HerdrCheck;

use crate::{
    herdr::Herdr,
    herdr_gate::{HerdrGate, Notice},
    supervise::Wake,
};

/// The identity and timing a heartbeat is published under.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// The machine this watcher serves, normalised.
    pub machine: MachineName,
    /// The watcher's process id.
    pub pid: u32,
    /// The watcher's version.
    pub version: String,
    /// When the process started, Unix milliseconds.
    pub started_at_ms: i64,
    /// The sweep interval, recorded in the heartbeat.
    pub sweep: Duration,
    /// How long each heartbeat lives: injectable so a test does not wait
    /// [`relay_core::health::HEARTBEAT_MIN_TTL`].
    pub ttl: Duration,
    /// What the OTLP push is doing, read on each heartbeat (FR-045).
    pub otlp: crate::otlp::export::StatusHandle,
}

/// The heartbeat's running state.
#[derive(Debug)]
struct Beat {
    config: HeartbeatConfig,
    last_sweep_ms: Option<i64>,
    /// A connection error was ridden out since the last write.
    redis_flapped: bool,
    last_error: Option<HeartbeatError>,
    /// Whether the "cannot write the heartbeat" line was printed for this
    /// condition.
    write_failing: bool,
    /// When the record was last written, Unix milliseconds: a burst of message
    /// wakes writes it once per [`MIN_WRITE_GAP`], not once per tick.
    last_write_ms: Option<i64>,
}

/// The fewest time between two heartbeat writes that follow message wakes. A
/// sweep always writes, so the record is never older than a sweep interval plus
/// one tick; this only keeps a busy bus from adding a request to every tick.
const MIN_WRITE_GAP: Duration = Duration::from_secs(1);

/// The watcher's heartbeat, herdr gate and registration-refresh state.
///
/// The default publishes no heartbeat (`--once`, and every test that does not
/// ask for one).
#[derive(Debug)]
pub struct Liveness {
    beat: Option<Beat>,
    gate: HerdrGate,
    /// Registered leads with a pane on this machine that the last tick looked at.
    pub leads_watched: u32,
    /// When each lead's liveness was last checked for the registration refresh.
    pub(crate) touch_checked: HashMap<AgentName, Instant>,
    /// A herdr call failed to decode and the check has not run since.
    decode_failure_pending: bool,
    /// The herdr check outstanding on its own thread, if any.
    running: Option<std::sync::mpsc::Receiver<HerdrCheck>>,
}

impl Default for Liveness {
    fn default() -> Self {
        Self {
            beat: None,
            gate: HerdrGate::new(crate::PLACE_REFRESH_EVERY),
            leads_watched: 0,
            touch_checked: HashMap::new(),
            decode_failure_pending: false,
            running: None,
        }
    }
}

/// How a failed tick is recorded in a heartbeat.
fn error_kind(error: &relay_core::Error) -> HeartbeatErrorKind {
    if error.is_connection() {
        HeartbeatErrorKind::RedisUnreachable
    } else {
        HeartbeatErrorKind::PollFailed
    }
}

impl Liveness {
    /// A watcher that publishes a heartbeat as `config` says.
    #[must_use]
    pub fn publishing(config: HeartbeatConfig) -> Self {
        Self {
            beat: Some(Beat {
                config,
                last_sweep_ms: None,
                redis_flapped: false,
                last_error: None,
                write_failing: false,
                last_write_ms: None,
            }),
            ..Self::default()
        }
    }

    /// Advances the herdr check without ever waiting for herdr: collects the
    /// result of a check that finished, and starts one on its own thread when one
    /// is due and none is running. A hung herdr therefore costs the loop
    /// nothing: the check stays outstanding (bounded by its own budget) and
    /// delivery goes on with the last status. Shows the notification, once per
    /// failing episode, through `herdr`.
    pub fn check_herdr(&mut self, herdr: &impl Herdr) {
        // A decode failure that arrives while a check is too soon is kept for the
        // next one, not lost.
        self.decode_failure_pending |= herdr.take_decode_failure();
        let now = Instant::now();
        if let Some(rx) = &self.running {
            match rx.try_recv() {
                Ok(found) => {
                    self.running = None;
                    if let Some(Notice { title, body }) = self.gate.record(now, found)
                        && let Err(e) = herdr.notify(&title, &body)
                    {
                        eprintln!(
                            "relay-watcher: could not show the herdr check notification: {e}"
                        );
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.running = None,
            }
        }
        if self.running.is_none() && self.gate.due(now, self.decode_failure_pending) {
            self.decode_failure_pending = false;
            let job = herdr.check_job();
            let (tx, rx) = std::sync::mpsc::channel();
            match std::thread::Builder::new()
                .name("relay-watcher-herdr-check".to_owned())
                .spawn(move || {
                    // The receiver is gone only when the watcher is.
                    let _ = tx.send(job());
                }) {
                Ok(_) => self.running = Some(rx),
                Err(e) => eprintln!("relay-watcher: could not start the herdr check: {e}"),
            }
        }
    }

    /// Whether the watcher must type into no pane: herdr's output is in a shape
    /// ix-relay cannot read (FR-036).
    #[must_use]
    pub fn delivery_blocked(&self) -> bool {
        self.gate.blocks_delivery()
    }

    /// Ends a tick that ran for `wake` and returned `result`: a failure is
    /// recorded for the next heartbeat and passed on, a success publishes one.
    ///
    /// # Errors
    ///
    /// `result`'s own error, or a connection-level failure writing the heartbeat
    /// (the tick is then a failed one: the record is not refreshed, and the
    /// connection is dropped and redialled like any other lost one).
    pub fn finish_tick(
        &mut self,
        relay: &mut Relay,
        wake: Wake,
        result: relay_core::Result<()>,
    ) -> relay_core::Result<()> {
        self.finish_tick_at(relay, wake, result, crate::now_ms())
    }

    /// [`Liveness::finish_tick`] at a given time.
    fn finish_tick_at(
        &mut self,
        relay: &mut Relay,
        wake: Wake,
        result: relay_core::Result<()>,
        now_ms: i64,
    ) -> relay_core::Result<()> {
        let leads = self.leads_watched;
        let herdr = self.gate.status().clone();
        let Some(beat) = self.beat.as_mut() else {
            return result;
        };
        if let Err(e) = result {
            beat.record_failure(&e, now_ms);
            return Err(e);
        }
        if wake == Wake::Sweep {
            beat.last_sweep_ms = Some(now_ms);
        } else if beat.last_write_ms.is_some_and(|at| {
            now_ms.saturating_sub(at) < i64::try_from(MIN_WRITE_GAP.as_millis()).unwrap_or(0)
        }) {
            return Ok(());
        }
        let heartbeat = beat.snapshot(now_ms, leads, &herdr);
        match relay.write_heartbeat(&heartbeat, beat.config.ttl) {
            Ok(()) => {
                beat.last_write_ms = Some(now_ms);
                beat.redis_flapped = false;
                if beat.write_failing {
                    eprintln!("relay-watcher: heartbeat writes work again");
                }
                beat.write_failing = false;
                Ok(())
            }
            Err(e) if e.is_connection() => {
                beat.record_failure(&e, now_ms);
                Err(e)
            }
            Err(e) => {
                // A key the heartbeat cannot be written to (the wrong type, an
                // ACL) costs the heartbeat, not the deliveries.
                if !beat.write_failing {
                    eprintln!("relay-watcher: cannot write its heartbeat: {e}");
                }
                beat.write_failing = true;
                beat.record_failure(&e, now_ms);
                Ok(())
            }
        }
    }
}

impl Beat {
    /// Notes a failed tick for the next heartbeat.
    fn record_failure(&mut self, error: &relay_core::Error, now_ms: i64) {
        self.redis_flapped |= error.is_connection();
        self.last_error = Some(HeartbeatError {
            kind: error_kind(error),
            at_ms: now_ms,
        });
    }

    /// The record for a tick that just succeeded at `now_ms`.
    fn snapshot(&self, now_ms: i64, leads: u32, herdr: &HerdrCheck) -> WatcherHeartbeat {
        WatcherHeartbeat {
            machine: self.config.machine.clone(),
            pid: self.config.pid,
            version: self.config.version.clone(),
            started_at_ms: self.config.started_at_ms,
            last_tick_ms: now_ms,
            last_sweep_ms: self.last_sweep_ms,
            sweep_secs: self.config.sweep.as_secs(),
            // A check that could not run is unknown, not a problem: the same
            // meaning `doctor` gives it locally.
            herdr_ok: !herdr.is_problem(),
            herdr_blocks_delivery: herdr.blocks_delivery(),
            herdr_problem: herdr
                .is_problem()
                .then(|| herdr.problem())
                .flatten()
                .map(|p| p.chars().take(MAX_PROBLEM_CHARS).collect()),
            redis_ok: !self.redis_flapped,
            leads_watched: leads,
            otlp: Some(self.config.otlp.get()),
            last_error: self.last_error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use relay_core::health::HeartbeatRead;
    use relay_test_support::Bus;

    use super::*;
    use crate::test_support::{connection_error, data_error};
    use relay_core::PaneId;

    fn config(ttl: Duration) -> HeartbeatConfig {
        HeartbeatConfig {
            machine: "hub".parse().unwrap(),
            pid: 4242,
            version: "9.9.9".into(),
            started_at_ms: 1_000,
            sweep: Duration::from_secs(15),
            ttl,
            otlp: crate::otlp::export::StatusHandle::disabled(),
        }
    }

    fn read(bus: &mut Bus) -> Option<WatcherHeartbeat> {
        match bus.relay.heartbeat(&"hub".parse().unwrap()).unwrap()? {
            HeartbeatRead::Live(hb) => Some(*hb),
            HeartbeatRead::Undecodable => None,
        }
    }

    /// Tracing: TC-028-01
    /// ACs: FR-028-AC-1
    ///
    /// Every good tick refreshes the record: its last-tick time is the tick's
    /// own, whether or not a message was involved, and a sweep stamps the
    /// last-sweep time while a retry does not clear it.
    #[test]
    fn tc_028_01_every_good_tick_refreshes_the_heartbeat() {
        let Some(mut bus) = Bus::connect("beat_refresh") else {
            return;
        };
        let mut live = Liveness::publishing(config(Duration::from_secs(60)));
        live.leads_watched = 3;
        let relay = &mut bus.another_connection();
        live.finish_tick_at(relay, Wake::Sweep, Ok(()), 10_000)
            .unwrap();
        let first = read(&mut bus).unwrap();
        assert_eq!(
            (first.last_tick_ms, first.last_sweep_ms, first.leads_watched),
            (10_000, Some(10_000), 3)
        );
        assert_eq!((first.pid, first.version.as_str()), (4242, "9.9.9"));
        assert!(first.redis_ok && first.herdr_ok && first.last_error.is_none());

        live.finish_tick_at(relay, Wake::Retry, Ok(()), 12_000)
            .unwrap();
        let second = read(&mut bus).unwrap();
        assert_eq!(
            (second.last_tick_ms, second.last_sweep_ms),
            (12_000, Some(10_000))
        );
    }

    /// Tracing: TC-028-02
    /// ACs: FR-028-AC-2
    ///
    /// The record has the expiry it was given, and a failed tick leaves it
    /// unrefreshed so it expires while failures persist.
    #[test]
    fn tc_028_02_a_failed_tick_leaves_the_heartbeat_to_expire() {
        let Some(mut bus) = Bus::connect("beat_expire") else {
            return;
        };
        let mut live = Liveness::publishing(config(Duration::from_millis(300)));
        let relay = &mut bus.another_connection();
        live.finish_tick_at(relay, Wake::Sweep, Ok(()), 1).unwrap();
        assert!(read(&mut bus).is_some());
        let err = live
            .finish_tick_at(relay, Wake::Sweep, Err(connection_error()), 2)
            .unwrap_err();
        assert!(err.is_connection());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while read(&mut bus).is_some() {
            assert!(std::time::Instant::now() < deadline, "never expired");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Tracing: TC-028-03
    /// ACs: FR-028-AC-3
    ///
    /// After failed ticks the next good one publishes again, the same process
    /// (same pid, same start time) reporting the flap and the last error, which
    /// stays after recovery; the next good tick after that is clean of the flap.
    #[test]
    fn tc_028_03_the_heartbeat_returns_after_an_outage_and_reports_it() {
        let Some(mut bus) = Bus::connect("beat_outage") else {
            return;
        };
        let mut live = Liveness::publishing(config(Duration::from_secs(60)));
        let relay = &mut bus.another_connection();
        live.finish_tick_at(relay, Wake::Sweep, Err(connection_error()), 100)
            .unwrap_err();
        live.finish_tick_at(relay, Wake::Sweep, Err(data_error()), 200)
            .unwrap_err();
        assert!(read(&mut bus).is_none());
        live.finish_tick_at(relay, Wake::Sweep, Ok(()), 300)
            .unwrap();
        let back = read(&mut bus).unwrap();
        assert_eq!((back.pid, back.started_at_ms), (4242, 1_000));
        assert!(!back.redis_ok, "the flap between heartbeats is reported");
        let error = back.last_error.unwrap();
        assert_eq!(
            (error.kind, error.at_ms),
            (HeartbeatErrorKind::PollFailed, 200)
        );
        live.finish_tick_at(relay, Wake::Sweep, Ok(()), 400)
            .unwrap();
        assert!(read(&mut bus).unwrap().redis_ok);
    }

    /// A burst of message wakes writes the record once a second, and a sweep
    /// always writes.
    #[test]
    fn message_wakes_within_a_second_do_not_rewrite_the_heartbeat_but_a_sweep_does() {
        let Some(mut bus) = Bus::connect("beat_gap") else {
            return;
        };
        let mut live = Liveness::publishing(config(Duration::from_secs(60)));
        let relay = &mut bus.another_connection();
        live.finish_tick_at(relay, Wake::Message, Ok(()), 10_000)
            .unwrap();
        live.finish_tick_at(relay, Wake::Message, Ok(()), 10_400)
            .unwrap();
        assert_eq!(read(&mut bus).unwrap().last_tick_ms, 10_000);
        live.finish_tick_at(relay, Wake::Message, Ok(()), 11_100)
            .unwrap();
        assert_eq!(read(&mut bus).unwrap().last_tick_ms, 11_100);
        live.finish_tick_at(relay, Wake::Sweep, Ok(()), 11_200)
            .unwrap();
        assert_eq!(read(&mut bus).unwrap().last_tick_ms, 11_200);
    }

    /// A heartbeat the watcher cannot write for a reason that is not the
    /// connection (here a Redis user that may not `SET`: `NOPERM`) costs the
    /// heartbeat, not the tick: the tick stays a good one and nothing is
    /// redialled, so delivery goes on. A connection error there still fails the
    /// tick (`tc_028_02`).
    #[test]
    fn a_heartbeat_that_cannot_be_written_for_a_non_connection_reason_does_not_fail_the_tick() {
        let Some(mut bus) = Bus::connect("beat_noperm") else {
            return;
        };
        let user = format!("hbtest{}", std::process::id());
        let mut setuser = redis::cmd("ACL");
        for arg in ["SETUSER", &user, "on", ">pw", "~*", "+@all", "-set"] {
            setuser.arg(arg);
        }
        setuser.exec(&mut bus.raw).unwrap();
        let url = bus
            .url
            .replacen("redis://", &format!("redis://{user}:pw@"), 1);
        let mut relay = relay_core::Relay::connect(&url, Duration::from_secs(2))
            .unwrap()
            .with_prefix(bus.prefix.clone());
        let mut live = Liveness::publishing(config(Duration::from_secs(60)));
        let raw_error = relay
            .write_heartbeat(&sample_heartbeat(), Duration::from_secs(60))
            .unwrap_err();
        assert!(
            !raw_error.is_connection(),
            "premise: NOPERM is a data error: {raw_error}"
        );
        let outcome = live.finish_tick_at(&mut relay, Wake::Sweep, Ok(()), 1);
        redis::cmd("ACL")
            .arg("DELUSER")
            .arg(&user)
            .exec(&mut bus.raw)
            .unwrap();
        assert!(outcome.is_ok(), "{outcome:?}");
        assert!(read(&mut bus).is_none(), "nothing was written");
    }

    fn sample_heartbeat() -> WatcherHeartbeat {
        Beat {
            config: config(Duration::from_secs(60)),
            last_sweep_ms: None,
            redis_flapped: false,
            last_error: None,
            write_failing: false,
            last_write_ms: None,
        }
        .snapshot(1, 0, &HerdrCheck::NotChecked)
    }

    #[test]
    fn a_watcher_that_does_not_publish_writes_no_heartbeat() {
        let Some(mut bus) = Bus::connect("beat_none") else {
            return;
        };
        let mut live = Liveness::default();
        let relay = &mut bus.another_connection();
        live.finish_tick_at(relay, Wake::Sweep, Ok(()), 1).unwrap();
        assert!(read(&mut bus).is_none());
    }

    struct Noisy {
        check: HerdrCheck,
        notes: RefCell<Vec<String>>,
    }

    impl Herdr for Noisy {
        fn get(
            &self,
            _: &PaneId,
        ) -> Result<Option<crate::herdr::AgentGet>, crate::herdr::ProcessError> {
            Ok(None)
        }
        fn notify(&self, title: &str, body: &str) -> Result<(), crate::herdr::ProcessError> {
            self.notes.borrow_mut().push(format!("{title}: {body}"));
            Ok(())
        }
        fn check(&self) -> HerdrCheck {
            self.check.clone()
        }
    }

    /// Runs the check to completion (it runs on its own thread).
    fn settle(live: &mut Liveness, herdr: &impl Herdr) {
        live.check_herdr(herdr);
        let deadline = Instant::now() + Duration::from_secs(5);
        while live.running.is_some() {
            assert!(Instant::now() < deadline, "the check never finished");
            std::thread::sleep(Duration::from_millis(2));
            live.check_herdr(herdr);
        }
    }

    /// The herdr result reaches the heartbeat, and the notification is raised
    /// through the herdr the watcher already uses.
    #[test]
    fn a_failing_herdr_check_notifies_once_and_shows_in_the_heartbeat() {
        let Some(mut bus) = Bus::connect("beat_herdr") else {
            return;
        };
        let herdr = Noisy {
            check: HerdrCheck::Missing {
                reason: "no such file".into(),
            },
            notes: RefCell::default(),
        };
        let mut live = Liveness::publishing(config(Duration::from_secs(60)));
        settle(&mut live, &herdr);
        live.check_herdr(&herdr);
        assert_eq!(herdr.notes.borrow().len(), 1);
        assert!(herdr.notes.borrow()[0].contains("no such file"));
        let relay = &mut bus.another_connection();
        live.finish_tick_at(relay, Wake::Sweep, Ok(()), 1).unwrap();
        let beat = read(&mut bus).unwrap();
        assert!(!beat.herdr_ok);
        assert!(beat.herdr_problem.unwrap().contains("no such file"));
    }

    /// A herdr with an idle agent in every pane and a scripted check, recording
    /// what is typed.
    struct Typer {
        check: HerdrCheck,
        typed: RefCell<Vec<String>>,
    }

    impl Herdr for Typer {
        fn get(
            &self,
            _: &PaneId,
        ) -> Result<Option<crate::herdr::AgentGet>, crate::herdr::ProcessError> {
            Ok(Some(crate::herdr::AgentGet {
                status: crate::herdr::AgentStatus::Idle,
                state_change_seq: 1,
                session_id: Some("s1".parse().unwrap()),
            }))
        }
        fn notify(&self, _: &str, _: &str) -> Result<(), crate::herdr::ProcessError> {
            Ok(())
        }
        fn check(&self) -> HerdrCheck {
            self.check.clone()
        }
    }

    impl crate::deliver::Deliverer for Typer {
        fn deliver(&self, _: &PaneId, text: &str) -> Result<(), crate::deliver::DeliverError> {
            self.typed.borrow_mut().push(text.to_owned());
            Ok(())
        }
    }

    /// Tracing: TC-036-07
    /// ACs: FR-036-AC-6
    ///
    /// Through a whole tick: while herdr's output is in a shape ix-relay cannot
    /// read the watcher types into no pane and the mail stays queued; with a
    /// herdr that passes, the same mail is delivered. An advisory finding
    /// (herdr missing) does not stop it.
    #[test]
    fn tc_036_07_a_tick_types_into_no_pane_while_the_shape_is_bad_and_delivers_otherwise() {
        let Some(mut bus) = Bus::connect("beat_blocked") else {
            return;
        };
        for name in ["receiver", "sender"] {
            bus.relay
                .register(&relay_core::Registration {
                    name: name.parse().unwrap(),
                    machine: "node-a".parse().unwrap(),
                    kind: "claude".into(),
                    session_id: Some("s1".parse().unwrap()),
                    cwd: None,
                    herdr_pane: Some(format!("w1:{name}").parse().unwrap()),
                    place: None,
                    last_seen_ms: crate::now_ms(),
                })
                .unwrap();
        }
        bus.relay
            .send(&relay_core::Message {
                from: "sender".parse().unwrap(),
                to: "receiver".parse().unwrap(),
                text: "hello".into(),
                sent_at_ms: 1,
            })
            .unwrap();
        let receiver: relay_core::AgentName = "receiver".parse().unwrap();

        let run = |bus: &mut Bus, check: HerdrCheck| {
            let herdr = Typer {
                check,
                typed: RefCell::default(),
            };
            let mut state = crate::WatcherState::default();
            settle(&mut state.live, &herdr);
            crate::tick(
                &mut bus.relay,
                &herdr,
                &"node-a".parse().unwrap(),
                &mut state,
                &mut |_| {},
            )
            .unwrap();
            herdr.typed.into_inner()
        };
        let blocked = run(
            &mut bus,
            HerdrCheck::ShapeMismatch {
                command: "agent get",
                detail: "no agent_session".into(),
            },
        );
        assert!(blocked.is_empty(), "typed while blocked: {blocked:?}");
        assert_eq!(
            bus.relay.inbox_len(&receiver).unwrap(),
            1,
            "mail stays queued"
        );

        let advisory = run(
            &mut bus,
            HerdrCheck::Missing {
                reason: "not on PATH".into(),
            },
        );
        assert_eq!(
            advisory.len(),
            1,
            "an advisory finding delivers: {advisory:?}"
        );
        assert_eq!(bus.relay.inbox_len(&receiver).unwrap(), 0);
    }

    /// A herdr whose check hangs costs the loop nothing: `check_herdr` returns at
    /// once, the check stays outstanding off the loop, and no second one starts
    /// while it is.
    #[test]
    fn a_hung_herdr_check_does_not_block_the_loop() {
        struct Hung {
            release: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
            jobs: std::cell::Cell<u32>,
        }
        impl Herdr for Hung {
            fn get(
                &self,
                _: &PaneId,
            ) -> Result<Option<crate::herdr::AgentGet>, crate::herdr::ProcessError> {
                Ok(None)
            }
            fn notify(&self, _: &str, _: &str) -> Result<(), crate::herdr::ProcessError> {
                Ok(())
            }
            fn check_job(&self) -> crate::herdr::CheckJob {
                self.jobs.set(self.jobs.get() + 1);
                let held = self.release.lock().unwrap().take();
                Box::new(move || {
                    if let Some(rx) = held {
                        let _ = rx.recv();
                    }
                    HerdrCheck::Failed {
                        reason: "released".into(),
                    }
                })
            }
        }
        let (release, held) = std::sync::mpsc::channel();
        let herdr = Hung {
            release: std::sync::Mutex::new(Some(held)),
            jobs: std::cell::Cell::new(0),
        };
        let mut live = Liveness::default();
        let started = Instant::now();
        for _ in 0..5 {
            live.check_herdr(&herdr);
        }
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the loop waited"
        );
        assert_eq!(herdr.jobs.get(), 1, "one check outstanding at a time");
        assert!(live.running.is_some());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while live.running.is_some() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
            live.check_herdr(&herdr);
        }
        assert_eq!(live.gate.status().code(), "failed");
    }

    /// A herdr fixture as herdr 0.9.1 reports it: one claude pane whose harness
    /// has not reported a session, beside a healthy lead with one. The check runs
    /// for real over the fixture.
    struct RealShaped {
        typed: RefCell<Vec<(String, String)>>,
    }

    struct FixtureRunner;

    impl herdr_cli::ProcessRunner for FixtureRunner {
        fn run(
            &self,
            _: &str,
            args: &[&str],
            _: Duration,
        ) -> Result<herdr_cli::Output, crate::herdr::ProcessError> {
            let out = |stdout: &str| {
                Ok(herdr_cli::Output {
                    status_code: 0,
                    success: true,
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: Vec::new(),
                })
            };
            match args {
                ["--version"] => out("herdr 0.9.1\n"),
                ["agent", "list"] => {
                    out(relay_test_support::HERDR_LIST_ONE_CLAUDE_PANE_WITHOUT_A_SESSION)
                }
                ["agent", "get", _] => out(
                    r#"{"result":{"agent":{"agent":"claude","agent_status":"idle","state_change_seq":10}}}"#,
                ),
                other => Err(crate::herdr::ProcessError::Spawn {
                    program: "herdr".into(),
                    reason: format!("no fixture for {other:?}"),
                }),
            }
        }
    }

    impl Herdr for RealShaped {
        fn get(
            &self,
            pane: &PaneId,
        ) -> Result<Option<crate::herdr::AgentGet>, crate::herdr::ProcessError> {
            // p1 reports no session id; p2 reports the healthy lead's.
            Ok(Some(crate::herdr::AgentGet {
                status: crate::herdr::AgentStatus::Idle,
                state_change_seq: 1,
                session_id: (pane.as_str() == "w1:p2").then(|| "sess-lead-a".parse().unwrap()),
            }))
        }
        fn notify(&self, _: &str, _: &str) -> Result<(), crate::herdr::ProcessError> {
            Ok(())
        }
        fn check(&self) -> HerdrCheck {
            relay_ops::herdr_check::check(&FixtureRunner, Duration::from_secs(1))
        }
    }

    impl crate::deliver::Deliverer for RealShaped {
        fn deliver(&self, pane: &PaneId, text: &str) -> Result<(), crate::deliver::DeliverError> {
            self.typed
                .borrow_mut()
                .push((pane.to_string(), text.to_owned()));
            Ok(())
        }
    }

    /// Tracing: TC-036-07
    /// ACs: FR-036-AC-7
    ///
    /// The regression the re-review found: a claude pane with no `agent_session`
    /// (herdr's schema makes it optional) must not stop delivery on the machine.
    /// The check passes, the healthy lead's mail is typed into its pane, and the
    /// lead whose pane reports no session is refused by the per-lead guard (its
    /// mail stays queued). A check that required `agent_session` blocks both.
    #[test]
    fn tc_036_07_a_pane_without_a_session_does_not_stop_delivery_to_a_healthy_lead() {
        let Some(mut bus) = Bus::connect("beat_no_session") else {
            return;
        };
        for (name, pane, session) in [
            ("lead-a", "w1:p2", "sess-lead-a"),
            ("lead-b", "w1:p1", "sess-lead-b"),
            ("sender", "w9:p9", "sess-s"),
        ] {
            bus.relay
                .register(&relay_core::Registration {
                    name: name.parse().unwrap(),
                    machine: "node-a".parse().unwrap(),
                    kind: "claude".into(),
                    session_id: Some(session.parse().unwrap()),
                    cwd: None,
                    herdr_pane: Some(pane.parse().unwrap()),
                    place: None,
                    last_seen_ms: crate::now_ms(),
                })
                .unwrap();
        }
        for to in ["lead-a", "lead-b"] {
            bus.relay
                .send(&relay_core::Message {
                    from: "sender".parse().unwrap(),
                    to: to.parse().unwrap(),
                    text: format!("hello {to}"),
                    sent_at_ms: 1,
                })
                .unwrap();
        }
        let herdr = RealShaped {
            typed: RefCell::default(),
        };
        let mut state = crate::WatcherState::default();
        settle(&mut state.live, &herdr);
        assert!(!state.live.delivery_blocked(), "the check must pass");
        crate::tick(
            &mut bus.relay,
            &herdr,
            &"node-a".parse().unwrap(),
            &mut state,
            &mut |_| {},
        )
        .unwrap();
        let typed = herdr.typed.into_inner();
        assert_eq!(typed.len(), 1, "{typed:?}");
        assert_eq!(typed[0].0, "w1:p2");
        assert!(typed[0].1.contains("hello lead-a"));
        let b: relay_core::AgentName = "lead-b".parse().unwrap();
        assert_eq!(
            bus.relay.inbox_len(&b).unwrap(),
            1,
            "the guard keeps lead-b's mail"
        );
    }
}
