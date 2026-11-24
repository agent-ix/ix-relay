//! The host collector (FR-069) and its source health (FR-070).
//!
//! The collector measures this host natively (no shell, no ssh), keeps the
//! lane registry of the host, classifies each lane's pane tail, and fills the
//! `ix.fleet.*` series of FR-045 for the push. Every source runs under its own
//! time budget ([`bound`]) and reports its own health ([`health`]): a failing
//! source exports `probe_ok` 0 and leaves its measured series out, never an old
//! value, never an absent `probe_ok`.
//!
//! Seams left for the hook feed (FR-068, a later change): [`Collector::hook_feed_attached`]
//! makes the `hook_events` source readable, [`Collector::set_hook_signal`]
//! feeds `blocked`/`idle` events into the lane state, and
//! [`Collector::compilers`] is the compiler list a lane attribution will read.

pub mod bound;
pub mod health;
pub mod host;
pub mod lanes;
#[cfg(target_os = "macos")]
mod mac;
pub mod pane;
pub mod procs;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use relay_core::{MachineName, PaneId};

use self::{
    bound::{SOURCE_BUDGET, Slot},
    health::{ErrorClass, Health, Outcome, Profile, Source},
    host::{CpuSampler, CpuTicks, Memory},
    lanes::{HookSignal, Lane, MAX_LANES, Signals},
    pane::{Harness, PaneClass},
    procs::Compiler,
};
use crate::otlp::{
    registry::{Attrs, Registry},
    series::{
        FLEET_BUSY_RATIO, FLEET_CAPACITY_CONFIGURED, FLEET_CAPACITY_SLOTS, FLEET_LANE_BLOCKED,
        FLEET_LANE_DROPPED, FLEET_LANE_LAST_ACTIVITY, FLEET_LANE_STATE, FLEET_MEMORY_TOTAL,
        FLEET_MEMORY_USED, FLEET_NATIVE_WINDOWS, FLEET_PROBE_OK, FLEET_PROBE_UNAVAILABLE,
        FLEET_TEMPERATURE,
    },
};

/// How much of one source slot the whole pane scan may use: the scan checks the
/// time left before each read and caps the read at it, so it ends inside
/// [`SOURCE_BUDGET`] with its completed reads kept.
const PANE_SCAN_BUDGET: Duration = Duration::from_millis(400);

/// The longest one pane read may take, and the least time left worth starting one.
const PANE_READ_MAX: Duration = Duration::from_millis(300);
const PANE_READ_MIN: Duration = Duration::from_millis(20);

type Read<T> = Arc<dyn Fn() -> Result<T, ErrorClass> + Send + Sync>;
type PaneRead = Arc<dyn Fn(&PaneId, Duration) -> Result<String, ErrorClass> + Send + Sync>;

/// What the collector reads. [`Sources::native`] is this host; tests give
/// fixtures.
#[derive(Clone)]
pub struct Sources {
    /// Cumulative CPU ticks.
    pub cpu: Read<CpuTicks>,
    /// Memory.
    pub memory: Read<Memory>,
    /// Temperature in degrees Celsius.
    pub temperature: Read<f64>,
    /// Compiler processes.
    pub compilers: Read<Vec<Compiler>>,
    /// Native Windows agent processes; `None` off WSL.
    pub native_windows: Option<Read<usize>>,
    /// The tail of one pane's output.
    pub pane_tail: PaneRead,
}

impl Sources {
    /// This host's own sources.
    #[must_use]
    pub fn native() -> Self {
        Self {
            cpu: Arc::new(host::read_cpu_ticks),
            memory: Arc::new(host::read_memory),
            temperature: Arc::new(host::read_temperature),
            compilers: Arc::new(|| procs::read_compilers(procs::DEFAULT_COMPILERS)),
            native_windows: (cfg!(target_os = "linux") && procs::is_wsl())
                .then(|| Arc::new(procs::read_native_windows) as Read<usize>),
            pane_tail: Arc::new(read_pane_tail),
        }
    }
}

/// `herdr pane read <pane> --lines 80`: plain text, bounded by the runner.
fn read_pane_tail(pane: &PaneId, timeout: Duration) -> Result<String, ErrorClass> {
    use herdr_cli::{ProcessError, ProcessRunner, RealProcessRunner};
    let lines = pane::TAIL_LINES.to_string();
    let out = RealProcessRunner
        .run(
            "herdr",
            &["pane", "read", pane.as_str(), "--lines", &lines],
            timeout,
        )
        .map_err(|e| match e {
            ProcessError::Timeout { .. } => ErrorClass::Timeout,
            _ => ErrorClass::Unreadable,
        })?;
    if !out.success {
        return Err(ErrorClass::Unreadable);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(pane::bound_tail(&text).to_owned())
}

/// What the store told the collector at the last layout refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityRead {
    /// Not read yet.
    Unread,
    /// Read, and no capacity is entered for this host.
    NotEntered,
    /// Read: this many slots entered.
    Slots(u32),
    /// The read failed.
    Failed(ErrorClass),
}

/// What the heartbeat reports of the collector, shared with the thread that writes it.
#[derive(Debug, Clone, Default)]
pub struct StatusHandle(Arc<std::sync::Mutex<(Vec<String>, u32)>>);

impl StatusHandle {
    /// The failed source names and the registry lanes left out past the cap.
    #[must_use]
    pub fn get(&self) -> (Vec<String>, u32) {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn set(&self, failed: Vec<String>, dropped: u32) {
        if let Ok(mut g) = self.0.lock() {
            *g = (failed, dropped);
        }
    }
}

/// The host collector.
pub struct Collector {
    host: MachineName,
    profile: Profile,
    sources: Sources,
    health: Health,
    slots: HashMap<Source, Slot>,
    windows_slot: Slot,
    cpu: CpuSampler,
    ratio: Option<f64>,
    memory: Option<Memory>,
    temperature: Option<f64>,
    compilers: Vec<Compiler>,
    native_windows: Option<usize>,
    capacity: CapacityRead,
    lanes: Vec<Lane>,
    first_seen: HashMap<String, Instant>,
    classes: HashMap<String, (PaneClass, Instant)>,
    hooks: HashMap<String, (HookSignal, Instant)>,
    scan_from: usize,
    interval: Duration,
    status: StatusHandle,
    refresh_redis_ok: bool,
    push_redis_ok: bool,
}

impl Collector {
    /// A collector for `host` with `profile`, reading `sources` once per push `interval`.
    #[must_use]
    pub fn new(host: MachineName, profile: Profile, sources: Sources, interval: Duration) -> Self {
        let mut health = Health::default();
        // No hook feed is attached yet: the source is expected and not readable,
        // which is a standing failure and not silence (FR-070).
        health.record(Source::HookEvents, Outcome::Failed(ErrorClass::Unavailable));
        Self {
            host,
            profile,
            sources,
            health,
            slots: Source::ALL
                .into_iter()
                .map(|s| (s, Slot::default()))
                .collect(),
            windows_slot: Slot::default(),
            cpu: CpuSampler::default(),
            ratio: None,
            memory: None,
            temperature: None,
            compilers: Vec::new(),
            native_windows: None,
            capacity: CapacityRead::Unread,
            lanes: Vec::new(),
            first_seen: HashMap::new(),
            classes: HashMap::new(),
            hooks: HashMap::new(),
            scan_from: 0,
            interval,
            status: StatusHandle::default(),
            refresh_redis_ok: true,
            push_redis_ok: true,
        }
    }

    #[allow(
        dead_code,
        reason = "seam: the lane attribution of the hook feed change reads it"
    )]
    /// The compiler processes of the last sample: what a lane attribution reads.
    #[must_use]
    pub fn compilers(&self) -> &[Compiler] {
        &self.compilers
    }

    /// The sources that failed their last read.
    #[cfg(test)]
    #[must_use]
    pub fn failed_sources(&self) -> Vec<Source> {
        self.health.failed()
    }

    /// The handle the heartbeat reads: failed sources and dropped lanes.
    #[must_use]
    pub fn status(&self) -> StatusHandle {
        self.status.clone()
    }

    fn publish_status(&self) {
        let failed = self
            .health
            .failed()
            .into_iter()
            .map(|s| s.name().to_owned())
            .collect();
        let dropped = u32::try_from(self.lanes.len().saturating_sub(MAX_LANES)).unwrap_or(u32::MAX);
        self.status.set(failed, dropped);
    }

    #[allow(dead_code, reason = "seam: the hook feed change attaches itself here")]
    /// Marks the hook feed as running: `hook_events` reads ok.
    pub fn hook_feed_attached(&mut self, log: &mut impl FnMut(String)) {
        self.record(Source::HookEvents, Outcome::Ok, log);
    }

    #[allow(dead_code, reason = "seam: the hook feed change reports events here")]
    /// Feeds the newest `blocked` or `idle` hook event of lane `address`.
    pub fn set_hook_signal(&mut self, address: &str, signal: HookSignal, at: Instant) {
        self.hooks.insert(address.to_owned(), (signal, at));
    }

    /// The lanes of a layout refresh at `now`. A lane new to the registry is
    /// timed from now; one that left is forgotten.
    pub fn set_lanes(&mut self, now: Instant, lanes: Vec<Lane>) {
        for lane in &lanes {
            self.first_seen.entry(lane.address.clone()).or_insert(now);
        }
        let keep: std::collections::HashSet<&str> =
            lanes.iter().map(|l| l.address.as_str()).collect();
        self.first_seen.retain(|a, _| keep.contains(a.as_str()));
        self.classes.retain(|a, _| keep.contains(a.as_str()));
        self.hooks.retain(|a, _| keep.contains(a.as_str()));
        self.lanes = lanes;
        self.publish_status();
    }

    /// The outcome of the last layout read from herdr.
    pub fn set_herdr(&mut self, outcome: Outcome, log: &mut impl FnMut(String)) {
        self.record(Source::Herdr, outcome, log);
    }

    /// What the last read of the store's capacity record gave.
    pub fn set_capacity(&mut self, read: CapacityRead, log: &mut impl FnMut(String)) {
        self.capacity = read;
        let outcome = match read {
            CapacityRead::Slots(_) => Outcome::Ok,
            CapacityRead::Failed(class) => Outcome::Failed(class),
            // No capacity entered is a standing failure of the source (FR-070).
            CapacityRead::NotEntered | CapacityRead::Unread => {
                Outcome::Failed(ErrorClass::Unavailable)
            }
        };
        self.record(Source::Capacity, outcome, log);
    }

    /// Whether the store answered the layout refresh's reads.
    pub fn set_refresh_redis(&mut self, ok: bool, log: &mut impl FnMut(String)) {
        self.refresh_redis_ok = ok;
        self.record_redis(log);
    }

    /// Whether the store answered the push's reads.
    pub fn set_push_redis(&mut self, ok: bool, log: &mut impl FnMut(String)) {
        self.push_redis_ok = ok;
        self.record_redis(log);
    }

    fn record_redis(&mut self, log: &mut impl FnMut(String)) {
        let outcome = if self.refresh_redis_ok && self.push_redis_ok {
            Outcome::Ok
        } else {
            Outcome::Failed(ErrorClass::Unreadable)
        };
        self.record(Source::Redis, outcome, log);
    }

    fn record(&mut self, source: Source, outcome: Outcome, log: &mut impl FnMut(String)) {
        if let Some(line) = self.health.record(source, outcome) {
            log(line);
        }
        self.publish_status();
    }

    fn bounded<T: Send + 'static>(&self, source: Source, read: &Read<T>) -> Result<T, ErrorClass> {
        let read = Arc::clone(read);
        self.slots
            .get(&source)
            .cloned()
            .unwrap_or_default()
            .run(SOURCE_BUDGET, move || read())?
    }

    /// Reads every source the profile expects, each under its own budget: a
    /// slow or failing source costs the tick at most [`SOURCE_BUDGET`] and the
    /// others still report.
    pub fn sample(&mut self, now: Instant, log: &mut impl FnMut(String)) {
        if self.profile.expects(Source::Cpu) {
            let (outcome, ratio) = match self.bounded(Source::Cpu, &self.sources.cpu.clone()) {
                Ok(ticks) => match self.cpu.sample(now, ticks) {
                    Some(r) => (Outcome::Ok, Some(r)),
                    None => (Outcome::Pending, None),
                },
                Err(class) => (Outcome::Failed(class), None),
            };
            self.ratio = ratio;
            self.record(Source::Cpu, outcome, log);
        }
        if self.profile.expects(Source::Memory) {
            let got = self.bounded(Source::Memory, &self.sources.memory.clone());
            self.memory = got.ok();
            self.record(
                Source::Memory,
                got.map_or_else(Outcome::Failed, |_| Outcome::Ok),
                log,
            );
        }
        if self.profile.expects(Source::Temperature) {
            let got = self.bounded(Source::Temperature, &self.sources.temperature.clone());
            self.temperature = got.ok();
            self.record(
                Source::Temperature,
                got.map_or_else(Outcome::Failed, |_| Outcome::Ok),
                log,
            );
        }
        if self.profile.expects(Source::Compilers) {
            let got = self.bounded(Source::Compilers, &self.sources.compilers.clone());
            let outcome = got
                .as_ref()
                .map_or_else(|c| Outcome::Failed(*c), |_| Outcome::Ok);
            self.compilers = got.unwrap_or_default();
            self.record(Source::Compilers, outcome, log);
        }
        if let Some(read) = self.sources.native_windows.clone() {
            // Not a profile source: a failed read leaves the series out.
            self.native_windows = self
                .windows_slot
                .run(SOURCE_BUDGET, move || read())
                .ok()
                .and_then(Result::ok);
        }
        if self.profile.expects(Source::PaneTail) {
            self.scan_panes(now, log);
        }
    }

    /// Reads pane tails round-robin until the scan budget is used, and keeps each
    /// lane's class. The source fails when every read of a scan failed.
    fn scan_panes(&mut self, now: Instant, log: &mut impl FnMut(String)) {
        let targets: Vec<(String, PaneId, Harness)> = self
            .lanes
            .iter()
            .filter_map(|l| Some((l.address.clone(), l.pane.clone()?, l.harness)))
            .collect();
        if targets.is_empty() {
            self.record(Source::PaneTail, Outcome::Ok, log);
            return;
        }
        let start = self.scan_from % targets.len();
        let read = Arc::clone(&self.sources.pane_tail);
        let job_targets = targets.clone();
        let scanned = self
            .slots
            .get(&Source::PaneTail)
            .cloned()
            .unwrap_or_default()
            .run(SOURCE_BUDGET, move || {
                let began = Instant::now();
                let mut done = Vec::new();
                for offset in 0..job_targets.len() {
                    let left = PANE_SCAN_BUDGET.saturating_sub(began.elapsed());
                    if left < PANE_READ_MIN {
                        break;
                    }
                    let (address, pane, harness) =
                        &job_targets[(start + offset) % job_targets.len()];
                    let class = read(pane, left.min(PANE_READ_MAX))
                        .map(|tail| pane::classify(&tail, *harness));
                    done.push((address.clone(), class));
                }
                done
            });
        match scanned {
            Err(class) => self.record(Source::PaneTail, Outcome::Failed(class), log),
            Ok(done) => {
                self.scan_from = start.saturating_add(done.len());
                let mut any_ok = false;
                let mut first_err = None;
                for (address, result) in done {
                    match result {
                        Ok(class) => {
                            any_ok = true;
                            self.classes.insert(address, (class, now));
                        }
                        Err(class) => {
                            first_err.get_or_insert(class);
                            // An unread pane keeps no stale class.
                            self.classes.remove(&address);
                        }
                    }
                }
                let outcome = match (any_ok, first_err) {
                    (false, Some(class)) => Outcome::Failed(class),
                    _ => Outcome::Ok,
                };
                self.record(Source::PaneTail, outcome, log);
            }
        }
    }

    /// The signals of a lane at `now`. A pane class counts only while `pane_tail`
    /// reads ok and the class is younger than one scan interval: an old class is
    /// never exported as current (FR-070).
    fn signals(&self, address: &str, now: Instant) -> Signals {
        let pane = self.classes.get(address).copied().filter(|(_, at)| {
            self.health.ok(Source::PaneTail) && now.saturating_duration_since(*at) < self.interval
        });
        Signals {
            pane,
            hook: self.hooks.get(address).copied(),
        }
    }

    /// Adds the fleet series of this host to `registry`. A source that does not
    /// read ok leaves its measured series out, and every expected source has its
    /// `probe_ok`.
    pub fn fill(&self, registry: &mut Registry, now: Instant) {
        let host = self.host.as_str();
        let host_attr = || Attrs::new().with("host", host);
        for source in Source::ALL {
            let attrs = || {
                Attrs::new()
                    .with("host", host)
                    .with("source", source.name())
            };
            if self.profile.expects(source) {
                registry.gauge(&FLEET_PROBE_OK, attrs(), i64::from(self.health.ok(source)));
            } else {
                // Not expected, or declared absent: unavailable, never a failure.
                registry.gauge(&FLEET_PROBE_UNAVAILABLE, attrs(), 1);
            }
        }
        if self.health.ok(Source::Cpu)
            && let Some(ratio) = self.ratio
        {
            registry.gauge_f64(&FLEET_BUSY_RATIO, host_attr(), ratio);
        }
        if self.health.ok(Source::Memory)
            && let Some(m) = self.memory
        {
            registry.gauge(&FLEET_MEMORY_USED, host_attr(), clamp(m.used));
            registry.gauge(&FLEET_MEMORY_TOTAL, host_attr(), clamp(m.total));
        }
        if self.profile.expects(Source::Temperature)
            && self.health.ok(Source::Temperature)
            && let Some(t) = self.temperature
        {
            registry.gauge_f64(&FLEET_TEMPERATURE, host_attr(), t);
        }
        match self.capacity {
            CapacityRead::Slots(n) => {
                registry.gauge(&FLEET_CAPACITY_SLOTS, host_attr(), i64::from(n));
                registry.gauge(&FLEET_CAPACITY_CONFIGURED, host_attr(), 1);
            }
            _ => {
                registry.gauge(&FLEET_CAPACITY_CONFIGURED, host_attr(), 0);
            }
        }
        if let Some(n) = self.native_windows {
            registry.gauge(&FLEET_NATIVE_WINDOWS, host_attr(), clamp(n as u64));
        }
        for lane in self.lanes.iter().take(MAX_LANES) {
            let signals = self.signals(&lane.address, now);
            let state = signals.state(lane.herdr_status.as_deref());
            let lane_attrs = || Attrs::new().with("host", host).with("lane", &lane.address);
            registry.gauge(
                &FLEET_LANE_STATE,
                lane_attrs().with("state", state.name()),
                1,
            );
            let since = self
                .first_seen
                .get(&lane.address)
                .map_or(0, |at| now.saturating_duration_since(*at).as_secs());
            registry.gauge(&FLEET_LANE_LAST_ACTIVITY, lane_attrs(), clamp(since));
            if let Some(reason) = signals.blocked_reason() {
                registry.gauge(
                    &FLEET_LANE_BLOCKED,
                    lane_attrs().with("reason", reason.name()),
                    1,
                );
            }
        }
        let dropped = self.lanes.len().saturating_sub(MAX_LANES);
        registry.gauge(&FLEET_LANE_DROPPED, host_attr(), clamp(dropped as u64));
    }
}

fn clamp(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// A collector over fixtures, for this module's tests and the series catalogue test.
#[cfg(test)]
pub(crate) mod fixtures {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    pub(crate) const INTERVAL: Duration = Duration::from_secs(30);

    pub(crate) fn host() -> MachineName {
        "hub".parse().unwrap()
    }

    /// A `/proc/stat` fixture whose busy share is 40% per sample.
    fn ticks(n: &AtomicU64) -> CpuTicks {
        let k = n.fetch_add(1, Ordering::SeqCst);
        CpuTicks {
            busy: 400 * k,
            idle: 600 * k,
        }
    }

    /// Sources that all read fine; `failing` makes one fail with `class`.
    pub(crate) fn sources(failing: Option<(Source, ErrorClass)>) -> Sources {
        let counter = Arc::new(AtomicU64::new(1));
        let fail = move |source: Source| failing.filter(|(s, _)| *s == source).map(|(_, c)| c);
        Sources {
            cpu: Arc::new(move || fail(Source::Cpu).map_or_else(|| Ok(ticks(&counter)), Err)),
            memory: Arc::new(move || {
                fail(Source::Memory).map_or(Ok(Memory { used: 3, total: 8 }), Err)
            }),
            temperature: Arc::new(move || fail(Source::Temperature).map_or(Ok(61.5), Err)),
            compilers: Arc::new(move || {
                fail(Source::Compilers).map_or(
                    Ok(vec![Compiler {
                        name: "rustc".into(),
                        cwd: "/w".into(),
                    }]),
                    Err,
                )
            }),
            native_windows: Some(Arc::new(|| Ok(0))),
            pane_tail: Arc::new(move |_, _| {
                fail(Source::PaneTail).map_or(Ok("plain output\n".to_owned()), Err)
            }),
        }
    }

    pub(crate) fn lane(address: &str, pane: &str, kind: Harness) -> Lane {
        Lane {
            address: address.into(),
            pane: Some(pane.parse().unwrap()),
            harness: kind,
            herdr_status: Some("working".into()),
        }
    }

    /// Every source healthy after two samples a second apart.
    pub(crate) fn healthy(
        profile: Profile,
        failing: Option<(Source, ErrorClass)>,
    ) -> (Collector, Instant) {
        let mut c = Collector::new(host(), profile, sources(failing), INTERVAL);
        let t0 = Instant::now();
        let mut log = |_| {};
        c.set_lanes(t0, vec![lane("hub:QSL:plan", "w1:p1", Harness::Claude)]);
        c.hook_feed_attached(&mut log);
        let all = |s: Source| failing.filter(|(f, _)| *f == s).map(|(_, e)| e);
        c.set_herdr(
            all(Source::Herdr).map_or(Outcome::Ok, Outcome::Failed),
            &mut log,
        );
        c.set_capacity(
            all(Source::Capacity).map_or(CapacityRead::Slots(8), CapacityRead::Failed),
            &mut log,
        );
        c.set_refresh_redis(all(Source::Redis).is_none(), &mut log);
        if all(Source::HookEvents).is_some() {
            c.record(
                Source::HookEvents,
                Outcome::Failed(ErrorClass::Unavailable),
                &mut log,
            );
        }
        c.sample(t0, &mut log);
        let now = t0 + Duration::from_secs(2);
        c.sample(now, &mut log);
        (c, now)
    }

    /// A collector exporting every series the catalogue lists for fleet hosts.
    pub(crate) fn full() -> (Collector, Instant) {
        let profile = Profile::new(
            &Source::ALL
                .into_iter()
                .filter(|s| *s != Source::Herdr)
                .collect::<Vec<_>>(),
            &[Source::Herdr],
        );
        let (mut c, now) = healthy(profile, None);
        c.set_lanes(now, vec![lane("hub:QSL:plan", "w1:p1", Harness::Claude)]);
        c.classes
            .insert("hub:QSL:plan".into(), (PaneClass::Dialog, now));
        c.native_windows = Some(1);
        (c, now)
    }
}

#[cfg(test)]
mod tests {
    use ix_trace_rs::trace;
    use std::time::Instant;

    use super::{fixtures::*, *};
    use crate::otlp::{
        encode::{Resource, encode},
        registry::Value,
    };

    fn registry_of(c: &Collector, now: Instant) -> Registry {
        let mut r = Registry::new(2, 1);
        c.fill(&mut r, now);
        r
    }

    /// The integer value of the first point of `name` whose attributes include all of `want`.
    fn int(r: &Registry, name: &str, want: &[(&str, &str)]) -> Option<i64> {
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

    fn probe(r: &Registry, source: Source) -> Option<i64> {
        int(
            r,
            "ix.fleet.probe_ok",
            &[("host", "hub"), ("source", source.name())],
        )
    }

    fn unavailable(r: &Registry, source: Source) -> bool {
        int(
            r,
            "ix.fleet.probe_unavailable",
            &[("source", source.name())],
        )
        .is_some()
    }

    fn has(r: &Registry, name: &str) -> bool {
        r.points().iter().any(|p| p.instrument.name == name)
    }

    fn expected() -> Vec<Source> {
        Source::ALL
            .into_iter()
            .filter(|s| *s != Source::Temperature)
            .collect()
    }

    #[trace("FR-070-AC-1")]
    #[test]
    fn tc_070_01_healthy_fixtures_give_probe_ok_1_for_each_expected_source_only() {
        let (c, now) = healthy(Profile::new(&expected(), &[Source::Compilers]), None);
        let r = registry_of(&c, now);
        for s in expected() {
            if s == Source::Compilers {
                assert_eq!(probe(&r, s), None, "declared unavailable: no probe_ok");
                assert!(unavailable(&r, s));
            } else {
                assert_eq!(probe(&r, s), Some(1), "{}", s.name());
            }
        }
        assert_eq!(probe(&r, Source::Temperature), None);
        let ratio = r
            .points()
            .iter()
            .find_map(|p| match (&p.value, p.instrument.name) {
                (Value::Double(v), "ix.fleet.host.busy_ratio") => Some(*v),
                _ => None,
            });
        assert!((ratio.unwrap() - 0.4).abs() < 1e-9);
    }

    #[trace("FR-070-AC-2")]
    #[test]
    fn tc_070_02_each_source_failing_in_turn_exports_0_and_leaves_the_others_alone() {
        let classes = [
            ErrorClass::Unreadable,
            ErrorClass::Timeout,
            ErrorClass::Parse,
            ErrorClass::Unsupported,
        ];
        for (n, failing) in Source::ALL
            .into_iter()
            .filter(|s| *s != Source::Temperature)
            .enumerate()
        {
            let class = classes[n % classes.len()];
            let (c, now) = healthy(Profile::default(), Some((failing, class)));
            let r = registry_of(&c, now);
            for s in expected() {
                let want = i64::from(s != failing);
                assert_eq!(
                    probe(&r, s),
                    Some(want),
                    "{} failing, checking {}",
                    failing.name(),
                    s.name()
                );
            }
        }
    }

    #[trace("FR-070-AC-3")]
    #[test]
    fn tc_070_03_a_source_that_never_succeeded_is_present_with_0_on_the_first_push() {
        let c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
        let r = registry_of(&c, Instant::now());
        for s in expected() {
            assert_eq!(probe(&r, s), Some(0), "{}", s.name());
        }
        assert!(!has(&r, "ix.fleet.host.busy_ratio"));
    }

    #[trace("FR-070-AC-4")]
    #[test]
    fn tc_070_04_the_busy_ratio_is_absent_when_cpu_fails_after_a_good_read() {
        let (mut c, now) = healthy(Profile::default(), None);
        assert!(has(&registry_of(&c, now), "ix.fleet.host.busy_ratio"));
        c.sources.cpu = Arc::new(|| Err(ErrorClass::Unreadable));
        let mut lines = Vec::new();
        c.sample(now + Duration::from_secs(5), &mut |l| lines.push(l));
        let r = registry_of(&c, now);
        assert_eq!(probe(&r, Source::Cpu), Some(0));
        assert!(!has(&r, "ix.fleet.host.busy_ratio"), "no old value");
        assert_eq!(lines, ["collector source cpu failed (unreadable)"]);
        assert_eq!(c.failed_sources(), [Source::Cpu]);
    }

    #[trace("FR-070-AC-6")]
    #[test]
    fn tc_070_06_with_redis_down_redis_reads_0_and_the_other_sources_still_report() {
        let (mut c, now) = healthy(Profile::default(), None);
        c.set_push_redis(false, &mut |_| {});
        let r = registry_of(&c, now);
        assert_eq!(probe(&r, Source::Redis), Some(0));
        assert_eq!(probe(&r, Source::Cpu), Some(1));
        assert_eq!(probe(&r, Source::Memory), Some(1));
        assert!(has(&r, "ix.fleet.host.busy_ratio"));
    }

    #[trace("FR-070-AC-7", "FR-069-AC-4")]
    #[test]
    fn tc_070_07_temperature_is_unavailable_unless_expected_and_failed_when_unreadable() {
        // The profile omits temperature: unavailable, no probe_ok, no value, no failed source.
        let (c, now) = healthy(Profile::default(), None);
        let r = registry_of(&c, now);
        assert!(unavailable(&r, Source::Temperature));
        assert_eq!(probe(&r, Source::Temperature), None);
        assert!(!has(&r, "ix.fleet.host.temperature"));
        assert!(!c.failed_sources().contains(&Source::Temperature));
        // Expected and unreadable: probe_ok 0, still no value, and never a zero reading.
        let with_temp = Profile::new(&Source::ALL, &[]);
        let failing = Some((Source::Temperature, ErrorClass::Unreadable));
        let (c, now) = healthy(with_temp.clone(), failing);
        let r = registry_of(&c, now);
        assert_eq!(probe(&r, Source::Temperature), Some(0));
        assert!(!has(&r, "ix.fleet.host.temperature"));
        // Expected and readable: the value is exported.
        let (c, now) = healthy(with_temp, None);
        assert!(has(&registry_of(&c, now), "ix.fleet.host.temperature"));
    }

    #[trace("FR-070-AC-8")]
    #[test]
    fn tc_070_08_unset_capacity_is_a_standing_failure_and_entered_capacity_is_exported() {
        let (mut c, now) = healthy(Profile::default(), None);
        c.set_capacity(CapacityRead::NotEntered, &mut |_| {});
        let r = registry_of(&c, now);
        assert_eq!(probe(&r, Source::Capacity), Some(0));
        assert_eq!(int(&r, "ix.fleet.host.capacity_configured", &[]), Some(0));
        assert!(!has(&r, "ix.fleet.host.capacity_slots"));
        c.set_capacity(CapacityRead::Slots(8), &mut |_| {});
        let r = registry_of(&c, now);
        assert_eq!(probe(&r, Source::Capacity), Some(1));
        assert_eq!(int(&r, "ix.fleet.host.capacity_configured", &[]), Some(1));
        assert_eq!(int(&r, "ix.fleet.host.capacity_slots", &[]), Some(8));
    }

    #[trace("FR-069-AC-9")]
    #[test]
    fn tc_069_09_a_hung_source_costs_its_budget_and_the_others_still_report() {
        let (mut c, now) = healthy(Profile::default(), None);
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let hold = std::sync::Mutex::new(hold);
        c.sources.memory = Arc::new(move || {
            let _ = hold.lock().unwrap().recv();
            Ok(Memory { used: 1, total: 2 })
        });
        let started = Instant::now();
        let mut lines = Vec::new();
        c.sample(now + Duration::from_secs(3), &mut |l| lines.push(l));
        assert!(
            started.elapsed() < bound::SOURCE_BUDGET + Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
        let r = registry_of(&c, now);
        assert_eq!(probe(&r, Source::Memory), Some(0));
        assert_eq!(probe(&r, Source::Cpu), Some(1));
        assert_eq!(probe(&r, Source::Compilers), Some(1));
        assert_eq!(lines, ["collector source memory failed (timeout)"]);
        drop(release);
    }

    #[trace("FR-069-AC-8")]
    #[test]
    fn tc_069_08_matched_pane_text_reaches_no_series_and_no_log_line() {
        let marker = "SECRET-MARKER-9f3a";
        let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
        c.sources.pane_tail =
            Arc::new(move |_, _| Ok(format!("{marker} Do you want to proceed?\n")));
        let t0 = Instant::now();
        let mut lines = Vec::new();
        c.set_lanes(t0, vec![lane("hub:QSL:plan", "w1:p1", Harness::Claude)]);
        c.sample(t0, &mut |l| lines.push(l));
        let r = registry_of(&c, t0);
        assert_eq!(
            int(&r, "ix.fleet.lane.blocked", &[("reason", "dialog")]),
            Some(1)
        );
        let body = String::from_utf8(
            encode(&r, &Resource::new("s", None, "hub", "0", &[]))
                .unwrap()
                .body,
        )
        .unwrap();
        assert!(!body.contains(marker), "{body}");
        assert!(lines.iter().all(|l| !l.contains(marker)), "{lines:?}");
    }

    #[trace("FR-069-AC-11")]
    #[test]
    fn tc_069_11_every_registry_lane_has_state_and_last_activity_on_every_push() {
        let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
        let t0 = Instant::now();
        let addrs = ["hub:A:one", "hub:A:two", "hub:B:three"];
        c.set_lanes(
            t0,
            addrs
                .iter()
                .map(|a| lane(a, "w1:p1", Harness::Codex))
                .collect(),
        );
        // No sample, no hook event: the lanes are still all there.
        let r = registry_of(&c, t0 + Duration::from_secs(7));
        for a in addrs {
            assert_eq!(
                int(
                    &r,
                    "ix.fleet.lane.state",
                    &[("lane", a), ("state", "working")]
                ),
                Some(1)
            );
            assert_eq!(
                int(&r, "ix.fleet.lane.last_activity", &[("lane", a)]),
                Some(7)
            );
        }
        // A lane that leaves the registry leaves the series.
        c.set_lanes(t0, vec![lane("hub:A:one", "w1:p1", Harness::Codex)]);
        let r = registry_of(&c, t0);
        assert!(int(&r, "ix.fleet.lane.state", &[("lane", "hub:A:two")]).is_none());
    }

    #[trace("FR-069-AC-12")]
    #[test]
    fn tc_069_12_every_pane_class_exports_its_mapped_blocked_reason() {
        for harness in Harness::ALL {
            for class in PaneClass::ALL {
                let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
                let t0 = Instant::now();
                c.set_lanes(t0, vec![lane("hub:A:one", "w1:p1", harness)]);
                c.record(Source::PaneTail, Outcome::Ok, &mut |_| {});
                c.classes.insert("hub:A:one".into(), (class, t0));
                let r = registry_of(&c, t0);
                for reason in pane::BlockedReason::ALL {
                    let got = int(&r, "ix.fleet.lane.blocked", &[("reason", reason.name())]);
                    assert_eq!(
                        got.is_some(),
                        class.reason() == Some(reason),
                        "{class:?} {reason:?}"
                    );
                }
                let blocked = int(&r, "ix.fleet.lane.state", &[("state", "blocked")]).is_some();
                assert_eq!(blocked, class.reason().is_some());
            }
        }
        // Hook causes map to their reasons too.
        let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
        let t0 = Instant::now();
        c.set_lanes(t0, vec![lane("hub:A:one", "w1:p1", Harness::Claude)]);
        for reason in [
            pane::BlockedReason::Permission,
            pane::BlockedReason::Question,
        ] {
            c.set_hook_signal("hub:A:one", HookSignal::Blocked(reason), t0);
            let r = registry_of(&c, t0);
            assert_eq!(
                int(&r, "ix.fleet.lane.blocked", &[("reason", reason.name())]),
                Some(1)
            );
        }
    }

    /// A class that is a scan interval old, or any class while `pane_tail` reads 0,
    /// is not exported as current.
    #[trace("FR-070-AC-4")]
    #[test]
    fn tc_070_10_a_stale_pane_class_or_an_unhealthy_pane_tail_exports_no_blocked_value() {
        let (mut c, now) = healthy(Profile::default(), None);
        c.classes
            .insert("hub:QSL:plan".into(), (PaneClass::Dialog, now));
        let blocked = |c: &Collector, at: Instant| {
            int(
                &registry_of(c, at),
                "ix.fleet.lane.blocked",
                &[("reason", "dialog")],
            )
        };
        assert_eq!(blocked(&c, now), Some(1));
        assert_eq!(blocked(&c, now + INTERVAL), None, "one scan interval old");
        c.record(
            Source::PaneTail,
            Outcome::Failed(ErrorClass::Timeout),
            &mut |_| {},
        );
        assert_eq!(blocked(&c, now), None, "pane_tail reads 0");
    }

    /// A slow herdr cannot push the scan past its slot: each read is capped at the time
    /// left, the completed reads are kept and the next scan resumes after them.
    #[trace("FR-069-AC-9")]
    #[test]
    fn tc_069_09_a_slow_herdr_ends_the_scan_inside_its_budget_and_keeps_completed_reads() {
        let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
        c.sources.pane_tail = Arc::new(|_, timeout| {
            let took = Duration::from_millis(200);
            std::thread::sleep(took.min(timeout));
            if timeout < took {
                Err(ErrorClass::Timeout)
            } else {
                Ok("Do you want to proceed?\n".to_owned())
            }
        });
        let t0 = Instant::now();
        let lanes: Vec<Lane> = (0..6)
            .map(|n| lane(&format!("hub:W:t{n}"), &format!("w1:p{n}"), Harness::Claude))
            .collect();
        c.set_lanes(t0, lanes);
        let started = Instant::now();
        c.sample(t0, &mut |_| {});
        assert!(
            started.elapsed() < bound::SOURCE_BUDGET,
            "{:?}",
            started.elapsed()
        );
        assert!(
            c.health.ok(Source::PaneTail),
            "not a timeout of the whole scan"
        );
        assert!(!c.classes.is_empty(), "completed reads are kept");
        assert!(c.scan_from > 0 && c.scan_from < 6, "{}", c.scan_from);
    }

    /// The exporter half: the 51st lane is not folded into another, and the drop is counted.
    #[trace("FR-045-AC-31")]
    #[test]
    fn tc_045_31_the_51st_lane_is_dropped_and_counted_not_folded() {
        let mut c = Collector::new(host(), Profile::default(), sources(None), INTERVAL);
        let t0 = Instant::now();
        let lanes: Vec<Lane> = (0..51)
            .map(|n| lane(&format!("hub:W:t{n:02}"), "w1:p1", Harness::Claude))
            .collect();
        c.set_lanes(t0, lanes);
        let r = registry_of(&c, t0);
        let exported = r
            .points()
            .iter()
            .filter(|p| p.instrument.name == "ix.fleet.lane.state")
            .count();
        assert_eq!(exported, 50);
        assert_eq!(int(&r, "ix.fleet.lane.dropped", &[]), Some(1));
    }
}
