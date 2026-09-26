//! OTLP metrics push (FR-045): a small reusable exporter and the watcher's series.
//!
//! - [`registry`]: typed instruments and the [`registry::Registry`] a push fills.
//! - [`series`]: the catalogue of what the watcher emits.
//! - [`encode`]: a registry to an OTLP/HTTP JSON `ExportMetricsServiceRequest`.
//! - [`http`]: a std-only HTTP/1.1 POST over `TcpStream`, plain `http://` only.
//! - [`export`]: the bounded hand-off to one exporter thread, with timeouts,
//!   capped backoff and a status the heartbeat reports.
//! - [`panes`], [`relay`]: what fills the registry.
//!
//! The watcher only ever *connects out*. It binds no listener, and with no
//! endpoint configured it opens no socket and starts no thread
//! ([`Publisher::disabled`]).
//!
//! [`Publisher`] is the watcher's one handle on all of it: the tick records what
//! it did, the layout refresh hands it the panes, and once per interval it reads
//! the relay's health from Redis, builds a registry and submits it. The submit
//! never waits on the collector ([`export`]).

pub mod encode;
pub mod export;
pub mod http;
pub mod panes;
pub mod registry;
pub mod relay;
pub mod series;
#[cfg(test)]
pub(crate) mod test_collector;
#[cfg(test)]
pub(crate) mod test_herdr;

use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use relay_core::{MachineName, Registration, Relay, config::otlp::OtlpSettings};
use relay_ops::find::MachineView;

use self::{
    encode::Resource,
    export::{Exporter, Sender, Stats as ExportStats, StatusHandle},
    http::{HttpTransport, Timeouts, Transport},
    panes::PaneSeries,
    registry::Registry,
};

/// The `service.name` when nothing sets it.
pub const DEFAULT_SERVICE_NAME: &str = "ix-relay-watcher";

/// How long a pane reading is still published after the last successful layout
/// read: three refresh periods. Past it the panes are left out rather than
/// published under a fresh timestamp with names that may be long gone.
pub const PANES_MAX_AGE: Duration = Duration::from_secs(90);

/// Unix nanoseconds now.
fn unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

struct Active {
    exporter: Exporter,
    machine: MachineName,
    interval: Duration,
    next_due: Option<Instant>,
    panes: Option<(Instant, Vec<PaneSeries>)>,
    stats: relay::Stats,
    started_unix_nanos: u64,
    started_ms: i64,
    /// A failed read of the store's health data is logged once per episode.
    read_failed_reported: bool,
}

/// The watcher's handle on the exporter. Every method is a cheap no-op when it is
/// [disabled](Publisher::disabled).
pub struct Publisher {
    active: Option<Active>,
    status: StatusHandle,
}

impl Publisher {
    /// No endpoint: nothing is sent, no thread runs, no socket is opened.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            active: None,
            status: StatusHandle::disabled(),
        }
    }

    /// The publisher the settings ask for, announcing what it does on stderr: one
    /// line naming the collector (`host:port` only), one for each inherited setting
    /// that was ignored, and one counting skipped `OTEL_RESOURCE_ATTRIBUTES` pairs,
    /// the last only when a push is actually starting. With no OTLP setting at all
    /// it says nothing. `once` (a manual check) sends nothing and says nothing.
    #[must_use]
    pub fn from_settings(
        settings: &OtlpSettings,
        machine: &MachineName,
        version: &str,
        once: bool,
    ) -> Self {
        if once {
            return Self::disabled();
        }
        for note in &settings.ignored {
            eprintln!("relay-watcher: {note}");
        }
        let started = Self::start(settings, machine, version, |line| {
            eprintln!("relay-watcher: {line}");
        });
        match started {
            Ok(publisher) if publisher.enabled() => {
                if let Some(endpoint) = &settings.endpoint {
                    eprintln!(
                        "relay-watcher: pushing metrics to {} every {}s (OTLP/HTTP, JSON, plain http)",
                        endpoint.value.target(),
                        settings.interval_secs.value
                    );
                }
                if settings.resource_attributes_skipped > 0 {
                    eprintln!(
                        "relay-watcher: {} malformed pair(s) of {} ignored",
                        settings.resource_attributes_skipped,
                        relay_core::config::otlp::ENV_RESOURCE_ATTRIBUTES
                    );
                }
                publisher
            }
            Ok(_) => settings
                .ignored
                .first()
                .map_or_else(Self::disabled, |note| Self::ignoring(note.to_string())),
            Err(e) => {
                eprintln!("relay-watcher: cannot start the OTLP exporter thread: {e}");
                Self::disabled()
            }
        }
    }

    /// No exporter, because an inherited generic setting was ignored: the heartbeat
    /// carries `note` (a variable and a reason, never a value).
    #[must_use]
    pub fn ignoring(note: String) -> Self {
        Self {
            active: None,
            status: StatusHandle::ignoring(note),
        }
    }

    /// Starts pushing to `settings`' endpoint, or is [disabled](Self::disabled) when
    /// there is none. `log` receives the exporter thread's outage lines.
    ///
    /// # Errors
    ///
    /// The OS refusing to start the exporter thread.
    pub fn start(
        settings: &OtlpSettings,
        machine: &MachineName,
        version: &str,
        log: impl FnMut(String) + Send + 'static,
    ) -> std::io::Result<Self> {
        let Some(endpoint) = &settings.endpoint else {
            return Ok(Self::disabled());
        };
        let transport = HttpTransport::new(
            endpoint.value.clone(),
            settings.headers.value.clone(),
            Timeouts::default(),
            format!("ix-relay-watcher/{version}"),
        );
        let resource = Resource::new(
            DEFAULT_SERVICE_NAME,
            settings.service_name.as_deref(),
            machine.as_str(),
            version,
            &settings.resource_attributes,
        );
        Self::with_transport(
            resource,
            endpoint.value.target(),
            Duration::from_secs(settings.interval_secs.value),
            machine,
            Box::new(transport),
            log,
        )
    }

    /// [`Publisher::start`] over any [`Transport`].
    pub(crate) fn with_transport(
        resource: Resource,
        target: String,
        interval: Duration,
        machine: &MachineName,
        transport: Box<dyn Transport>,
        log: impl FnMut(String) + Send + 'static,
    ) -> std::io::Result<Self> {
        let stats = Arc::new(ExportStats::default());
        let status = StatusHandle::new(relay_core::health::OtlpStatus::Disabled, stats);
        let sender = Sender::new(resource, target, status.clone(), Box::new(log));
        let exporter = Exporter::start(sender, transport)?;
        Ok(Self {
            active: Some(Active {
                exporter,
                machine: machine.clone(),
                interval,
                next_due: None,
                panes: None,
                stats: relay::Stats::default(),
                started_unix_nanos: unix_nanos(),
                started_ms: i64::try_from(unix_nanos() / 1_000_000).unwrap_or(0),
                read_failed_reported: false,
            }),
            status,
        })
    }

    /// Whether anything is being pushed.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.active.is_some()
    }

    /// The state the heartbeat reports; shared with the exporter thread.
    #[must_use]
    pub fn status(&self) -> StatusHandle {
        self.status.clone()
    }

    /// Records the panes of a layout read at `now`: `view` joined with `regs`.
    pub fn set_panes(&mut self, now: Instant, view: &MachineView, regs: &[Registration]) {
        if let Some(a) = &mut self.active {
            a.panes = Some((now, panes::panes_of(&a.machine, view, regs)));
        }
    }

    /// Messages the watcher typed into a pane.
    pub fn delivered(&mut self, n: usize) {
        if let Some(a) = &mut self.active {
            a.stats.delivered(n);
        }
    }

    /// Messages a rejected prompt requeued.
    pub fn requeued(&mut self, n: usize) {
        if let Some(a) = &mut self.active {
            a.stats.requeued(n);
        }
    }

    /// What a sweep's reap of expired leases did.
    pub fn reaped(&mut self, redelivered: usize, dead_lettered: usize) {
        if let Some(a) = &mut self.active {
            a.stats.redelivered(redelivered);
            a.stats.dead_lettered(dead_lettered);
        }
    }

    /// One tick's duration.
    pub fn tick_took(&mut self, took: Duration) {
        if let Some(a) = &mut self.active {
            a.stats.tick(took);
        }
    }

    /// Builds the registry of a push from what was recorded and `health`,
    /// stamped `unix_nanos`. The panes are left out once older than
    /// [`PANES_MAX_AGE`] at `now`.
    pub(crate) fn build(&self, now: Instant, unix_nanos: u64, health: &relay::Health) -> Registry {
        let Some(a) = &self.active else {
            return Registry::new(unix_nanos, unix_nanos);
        };
        let mut registry = Registry::new(unix_nanos, a.started_unix_nanos);
        if let Some((at, panes)) = &a.panes
            && now.saturating_duration_since(*at) <= PANES_MAX_AGE
        {
            panes::fill(&mut registry, panes);
        }
        health.fill(&mut registry, &a.machine);
        a.stats.fill(&mut registry);
        registry
    }

    /// Pushes when the interval has passed since the last push (the first push is on
    /// the first call): reads the relay's health from the store, builds the registry
    /// and hands it to the exporter, which sends it on its own thread. The tick never
    /// waits for the collector.
    ///
    /// The read is at most `PUSH_REQUESTS` pipelined requests, and nothing that goes
    /// wrong in it reaches the caller: an error leaves those series out and is logged
    /// once per episode, so a metrics read can never skip the rest of the tick.
    pub fn publish_if_due(
        &mut self,
        relay: &mut Relay,
        regs: &[Registration],
        now: Instant,
        now_ms: i64,
        log: &mut impl FnMut(String),
    ) {
        let Some(a) = &mut self.active else {
            return;
        };
        if a.next_due.is_some_and(|due| now < due) {
            return;
        }
        a.next_due = Some(next_due_after(now, a.interval));
        let (health, failed) = relay::read(relay, &a.machine, regs, a.started_ms, now_ms);
        match failed {
            Some(e) if !std::mem::replace(&mut a.read_failed_reported, true) => log(format!(
                "cannot read the relay's health for the metrics push ({e}); those series are \
                 left out until it works"
            )),
            Some(_) => {}
            None => a.read_failed_reported = false,
        }
        let registry = self.build(now, unix_nanos(), &health);
        if let Some(a) = &self.active {
            a.exporter.submit(registry);
        }
    }
}

/// The most store requests one push adds to a tick: one pipelined read of the local
/// leads' queues and one read of the newest hook failures. FR-045 pins it.
#[cfg(test)]
pub(crate) const PUSH_REQUESTS: usize = 2;

/// When the push after one at `now` is due: `interval` later. The configuration
/// bounds the interval, but the arithmetic does not rely on it: a sum that would
/// overflow falls back to an hour.
#[must_use]
pub fn next_due_after(now: Instant, interval: Duration) -> Instant {
    now.checked_add(interval)
        .or_else(|| now.checked_add(Duration::from_secs(3600)))
        .unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use relay_core::config::{
        Resolved, Source,
        otlp::{OtlpEndpoint, OtlpHeaders},
    };

    use super::*;
    use crate::otlp::{
        encode::encode,
        series::CATALOGUE,
        test_collector::{FakeCollector, Recording, Reply},
        test_herdr::{hub, start, tab_renamed, view_from},
    };

    fn publisher() -> (Publisher, std::sync::mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let p = Publisher::with_transport(
            Resource::new(DEFAULT_SERVICE_NAME, None, "hub", "0", &[]),
            "c:4318".into(),
            Duration::ZERO,
            &hub(),
            Box::new(Recording(tx)),
            |_| {},
        )
        .unwrap();
        (p, rx)
    }

    fn body(p: &Publisher, now: Instant, health: &relay::Health) -> String {
        let registry = p.build(now, 2, health);
        String::from_utf8(
            encode(&registry, &Resource::new("s", None, "hub", "0", &[]))
                .unwrap()
                .body,
        )
        .unwrap()
    }

    fn settings(endpoint: Option<&str>) -> OtlpSettings {
        OtlpSettings {
            endpoint: endpoint.map(|e| Resolved {
                value: OtlpEndpoint::parse(e, true).unwrap(),
                source: Source::Env,
                origin: None,
            }),
            headers: Resolved {
                value: OtlpHeaders::parse("Authorization=Bearer s3cret").unwrap(),
                source: Source::Env,
                origin: None,
            },
            interval_secs: Resolved {
                value: 30,
                source: Source::Default,
                origin: None,
            },
            resource_attributes: vec![("deployment.environment".into(), "test".into())],
            resource_attributes_skipped: 0,
            service_name: Some("relay-under-test".into()),
            ignored: Vec::new(),
        }
    }

    /// The exported names, units, kinds and attribute keys are exactly the
    /// catalogue's, in order, and the catalogue is exactly FR-045's series table:
    /// adding, renaming or dropping a series or an attribute fails here until the
    /// code, the catalogue and the spec agree.
    ///
    /// Tracing: TC-045-11
    /// ACs: FR-045-AC-11
    #[test]
    fn tc_045_11_the_exported_series_are_the_catalogue_and_the_catalogue_is_the_spec() {
        let (mut p, _rx) = publisher();
        let now = Instant::now();
        let view = view_from(start());
        p.set_panes(now, &view, &[]);
        let health =
            relay::Health::for_test(&[("qsl-lead", 2, 30, 1, 4)], &[(("connect", "timeout"), 2)]);
        let registry = p.build(now, 2, &health);
        let mut exported: BTreeMap<&str, (Vec<&str>, &str)> = BTreeMap::new();
        for point in registry.points() {
            let keys: Vec<&str> = point.attrs.pairs().iter().map(|(k, _)| *k).collect();
            let entry = exported
                .entry(point.instrument.name)
                .or_insert((keys.clone(), point.instrument.unit));
            assert_eq!(
                entry.0, keys,
                "{} carries the same keys on every point",
                point.instrument.name
            );
        }
        let catalogue: BTreeMap<&str, (Vec<&str>, &str)> = CATALOGUE
            .iter()
            .map(|(i, keys)| (i.name, (keys.to_vec(), i.unit)))
            .collect();
        assert_eq!(exported, catalogue);

        let spec =
            include_str!("../../../../spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md");
        for (instrument, keys) in CATALOGUE {
            let kind = match instrument.kind {
                registry::Kind::Gauge => "gauge",
                registry::Kind::Sum => "sum",
                registry::Kind::Histogram { .. } => "histogram",
            };
            let attrs = if keys.is_empty() {
                "none".to_owned()
            } else {
                keys.join(", ")
            };
            let row = format!(
                "| `{}` | {kind} | `{}` | {attrs} |",
                instrument.name, instrument.unit
            );
            assert!(spec.contains(&row), "FR-045 has no series row {row:?}");
        }
        let rows = spec
            .lines()
            .filter(|l| l.starts_with("| `herdr.") || l.starts_with("| `ix.relay."))
            .count();
        assert_eq!(
            rows,
            CATALOGUE.len(),
            "a series in the spec that the code does not export"
        );
    }

    /// Through the watcher's own layout code and herdr's recorded output, on an
    /// injected clock: after a tab is renamed in herdr, the push built at the next
    /// refresh carries the new `tab_label` and no trace of the old one.
    ///
    /// Tracing: TC-045-05
    /// ACs: FR-045-AC-5
    #[test]
    fn tc_045_05_a_renamed_tab_shows_in_the_push_built_at_the_next_refresh() {
        let (mut p, _rx) = publisher();
        let t0 = Instant::now();
        p.set_panes(t0, &view_from(start()), &[]);
        let before = body(&p, t0, &relay::Health::default());
        assert!(before.contains(r#""stringValue":"tabB""#), "{before}");
        let t1 = t0 + Duration::from_secs(30);
        p.set_panes(t1, &view_from(tab_renamed()), &[]);
        let after = body(&p, t1, &relay::Health::default());
        assert!(after.contains(r#""stringValue":"tabB-renamed""#), "{after}");
        assert!(!after.contains(r#""stringValue":"tabB""#), "{after}");
    }

    /// Panes are published while their layout read is fresh and left out once it is
    /// older than three refresh periods, so a herdr that stopped answering does not
    /// keep publishing old names under new timestamps; the relay series continue.
    ///
    /// Tracing: TC-045-05
    /// ACs: FR-045-AC-5
    #[test]
    fn tc_045_05_stale_panes_are_left_out_after_three_refresh_periods() {
        let (mut p, _rx) = publisher();
        let t0 = Instant::now();
        p.set_panes(t0, &view_from(start()), &[]);
        let health = relay::Health::default();
        assert!(body(&p, t0 + PANES_MAX_AGE, &health).contains("herdr.pane.info"));
        let stale = body(&p, t0 + PANES_MAX_AGE + Duration::from_secs(1), &health);
        assert!(!stale.contains("herdr.pane.info"), "{stale}");
        assert!(
            stale.contains("ix.relay.messages"),
            "the relay series go on"
        );
    }

    /// With no endpoint nothing exists: no exporter, no thread, a `disabled`
    /// status, and every recording call is a no-op.
    ///
    /// Tracing: TC-045-01
    /// ACs: FR-045-AC-1
    #[test]
    fn tc_045_01_no_endpoint_starts_no_exporter() {
        let mut p = Publisher::start(&settings(None), &hub(), "0", |_| {}).unwrap();
        assert!(!p.enabled());
        assert_eq!(p.status().get(), relay_core::health::OtlpStatus::Disabled);
        p.set_panes(Instant::now(), &view_from(start()), &[]);
        p.delivered(3);
        p.tick_took(Duration::from_secs(1));
        assert!(
            p.build(Instant::now(), 2, &relay::Health::default())
                .points()
                .is_empty()
        );
        assert!(Publisher::disabled().active.is_none());
    }

    /// The real start path against a real listener: the request goes to
    /// `<endpoint>/v1/metrics` as JSON with the configured header, and the resource
    /// carries the service name, host, version and the extra attribute.
    ///
    /// Tracing: TC-045-10
    /// ACs: FR-045-AC-10
    #[test]
    fn tc_045_10_a_started_publisher_posts_json_with_its_resource_and_headers() {
        let collector = FakeCollector::start(vec![Reply::status(200)]);
        let p =
            Publisher::start(&settings(Some(&collector.url())), &hub(), "9.9.9", |_| {}).unwrap();
        assert!(p.enabled());
        let registry = p.build(Instant::now(), 2, &relay::Health::default());
        p.active.as_ref().unwrap().exporter.submit(registry);
        let seen = collector.recv();
        assert!(
            seen.head.starts_with("POST /v1/metrics HTTP/1.1"),
            "{}",
            seen.head
        );
        assert!(
            seen.head.contains("Content-Type: application/json"),
            "{}",
            seen.head
        );
        assert!(
            seen.head.contains("Authorization: Bearer s3cret"),
            "{}",
            seen.head
        );
        let json: serde_json::Value = serde_json::from_slice(&seen.body).unwrap();
        let attrs = &json["resourceMetrics"][0]["resource"]["attributes"];
        let get = |key: &str| {
            attrs
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["key"] == key)
                .map(|a| a["value"]["stringValue"].clone())
        };
        assert_eq!(get("service.name"), Some("relay-under-test".into()));
        assert_eq!(get("host.name"), Some("hub".into()));
        assert_eq!(get("service.version"), Some("9.9.9".into()));
        assert_eq!(get("deployment.environment"), Some("test".into()));
    }

    /// The next push is one interval on, and the arithmetic never panics on a huge
    /// interval (an hour is the fallback).
    ///
    /// Tracing: TC-045-22
    /// ACs: FR-045-AC-22
    #[test]
    fn tc_045_22_the_next_push_is_one_interval_on_and_a_huge_interval_never_panics() {
        let now = Instant::now();
        assert_eq!(
            next_due_after(now, Duration::from_secs(30)),
            now + Duration::from_secs(30)
        );
        for huge in [
            Duration::MAX,
            Duration::from_secs(u64::MAX),
            Duration::from_secs(i64::MAX as u64),
        ] {
            assert_eq!(next_due_after(now, huge), now + Duration::from_secs(3600));
        }
    }

    /// With an inherited setting that was ignored and no relay endpoint, nothing runs
    /// and the heartbeat state carries the note; `--once` sends and says nothing; with
    /// no setting at all the state is plain `disabled`.
    ///
    /// Tracing: TC-045-16
    /// ACs: FR-045-AC-16
    #[test]
    fn tc_045_16_an_ignored_inherited_setting_is_a_note_in_the_heartbeat_state() {
        let mut s = settings(None);
        let none = Publisher::from_settings(&s, &hub(), "0", false);
        assert!(!none.enabled());
        assert_eq!(
            none.status().get(),
            relay_core::health::OtlpStatus::Disabled
        );
        s.ignored.push(relay_core::config::otlp::Ignored {
            setting: "OTEL_EXPORTER_OTLP_ENDPOINT",
            reason: "https is not supported (plain http only)",
        });
        let noted = Publisher::from_settings(&s, &hub(), "0", false);
        assert!(!noted.enabled());
        assert_eq!(
            noted.status().get(),
            relay_core::health::OtlpStatus::Ignoring {
                note:
                    "OTEL_EXPORTER_OTLP_ENDPOINT ignored: https is not supported (plain http only)"
                        .into()
            }
        );
        let once = Publisher::from_settings(&s, &hub(), "0", true);
        assert_eq!(
            once.status().get(),
            relay_core::health::OtlpStatus::Disabled
        );
        assert!(!once.enabled());
    }
}
