//! Source health (FR-070): which sources a host is expected to provide, whether
//! the last read of each succeeded, and the one log line per transition.

use std::collections::{BTreeMap, BTreeSet};

use relay_core::config::FleetFile;

/// A collector source. The set is fixed by FR-070.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// CPU busy ratio.
    Cpu,
    /// Memory used and total.
    Memory,
    /// Hardware temperature.
    Temperature,
    /// Operator-entered server capacity.
    Capacity,
    /// The compiler process list.
    Compilers,
    /// Pane tails.
    PaneTail,
    /// Hook activity events.
    HookEvents,
    /// herdr's layout.
    Herdr,
    /// The relay's store.
    Redis,
}

impl Source {
    /// Every source, in the order FR-070 lists them.
    pub const ALL: [Source; 9] = [
        Source::Cpu,
        Source::Memory,
        Source::Temperature,
        Source::Capacity,
        Source::Compilers,
        Source::PaneTail,
        Source::HookEvents,
        Source::Herdr,
        Source::Redis,
    ];

    /// The name used in configuration and as the `source` attribute.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Source::Cpu => "cpu",
            Source::Memory => "memory",
            Source::Temperature => "temperature",
            Source::Capacity => "capacity",
            Source::Compilers => "compilers",
            Source::PaneTail => "pane_tail",
            Source::HookEvents => "hook_events",
            Source::Herdr => "herdr",
            Source::Redis => "redis",
        }
    }

    /// The source called `name`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// Why a read failed: the fixed set the log line may name, never content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// The read ran past its budget.
    Timeout,
    /// A file or call could not be read.
    Unreadable,
    /// This platform has no way to read it.
    Unsupported,
    /// What was read did not parse.
    #[cfg_attr(
        not(any(target_os = "linux", test)),
        allow(dead_code, reason = "only the Linux readers parse text")
    )]
    Parse,
    /// The thing to read is not there.
    Unavailable,
}

impl ErrorClass {
    /// The class as logged.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ErrorClass::Timeout => "timeout",
            ErrorClass::Unreadable => "unreadable",
            ErrorClass::Unsupported => "unsupported",
            ErrorClass::Parse => "parse",
            ErrorClass::Unavailable => "unavailable",
        }
    }
}

/// The result of one source read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Read within budget.
    Ok,
    /// Failed, by class.
    Failed(ErrorClass),
    /// Not yet readable, as a first CPU sample: reads 0 but is not an error.
    Pending,
}

/// The host profile (`[fleet]` in the configuration).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    expected: BTreeSet<Source>,
    unavailable: BTreeSet<Source>,
}

impl Default for Profile {
    /// Every source except `temperature` expected, none unavailable.
    fn default() -> Self {
        Self {
            expected: Source::ALL
                .into_iter()
                .filter(|s| *s != Source::Temperature)
                .collect(),
            unavailable: BTreeSet::new(),
        }
    }
}

impl Profile {
    /// A profile from the file's raw `[fleet]` values.
    ///
    /// # Errors
    ///
    /// A message naming the key and what is wrong with it.
    pub fn from_file(file: &FleetFile) -> Result<Self, String> {
        let mut profile = Self::default();
        if let Some(raw) = &file.expected_sources {
            profile.expected = names("expected_sources", raw)?;
        }
        if let Some(raw) = &file.unavailable_sources {
            profile.unavailable = names("unavailable_sources", raw)?;
        }
        Ok(profile)
    }

    #[cfg(test)]
    /// A profile with exactly these sources expected and these unavailable.
    #[must_use]
    pub fn new(expected: &[Source], unavailable: &[Source]) -> Self {
        Self {
            expected: expected.iter().copied().collect(),
            unavailable: unavailable.iter().copied().collect(),
        }
    }

    /// Whether `source` is exported through `probe_ok`: expected and not
    /// declared unavailable.
    #[must_use]
    pub fn expects(&self, source: Source) -> bool {
        self.expected.contains(&source) && !self.unavailable.contains(&source)
    }
}

fn names(key: &str, raw: &toml::Value) -> Result<BTreeSet<Source>, String> {
    let Some(items) = raw.as_array() else {
        return Err(format!("[fleet] {key} must be an array of source names"));
    };
    let mut out = BTreeSet::new();
    for item in items {
        let name = item
            .as_str()
            .ok_or_else(|| format!("[fleet] {key} must hold only strings"))?;
        let source = Source::parse(name).ok_or_else(|| {
            format!(
                "[fleet] {key} names an unknown source (one of: {})",
                Source::ALL.map(Source::name).join(", ")
            )
        })?;
        out.insert(source);
    }
    Ok(out)
}

/// The last outcome of each source, and the transitions to log.
#[derive(Debug, Clone, Default)]
pub struct Health {
    last: BTreeMap<Source, Outcome>,
}

impl Health {
    /// Records `outcome` for `source`. Returns the one log line owed when the
    /// source changed between ok and failed; it names the source and the error
    /// class and nothing read from the host.
    pub fn record(&mut self, source: Source, outcome: Outcome) -> Option<String> {
        let before = self.last.insert(source, outcome);
        match (before, outcome) {
            (Some(Outcome::Ok), Outcome::Failed(class)) | (None, Outcome::Failed(class)) => {
                Some(format!(
                    "collector source {} failed ({})",
                    source.name(),
                    class.name()
                ))
            }
            (Some(Outcome::Failed(_)), Outcome::Ok) => Some(format!(
                "collector source {} is readable again",
                source.name()
            )),
            _ => None,
        }
    }

    /// Whether the last read of `source` succeeded; false for one never read.
    #[must_use]
    pub fn ok(&self, source: Source) -> bool {
        self.last.get(&source) == Some(&Outcome::Ok)
    }

    /// The sources that failed their last read (a pending first read is not a
    /// failure), as the heartbeat of FR-028 lists them.
    #[must_use]
    pub fn failed(&self) -> Vec<Source> {
        self.last
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::Failed(_)))
            .map(|(s, _)| *s)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ix_trace_rs::trace;

    #[trace("FR-070-AC-5")]
    #[test]
    fn tc_070_05_a_transition_logs_one_line_with_the_class_and_nothing_else() {
        let mut h = Health::default();
        assert_eq!(h.record(Source::Cpu, Outcome::Ok), None);
        assert_eq!(h.record(Source::Cpu, Outcome::Ok), None);
        let line = h
            .record(Source::Cpu, Outcome::Failed(ErrorClass::Unreadable))
            .unwrap();
        assert_eq!(line, "collector source cpu failed (unreadable)");
        assert_eq!(
            h.record(Source::Cpu, Outcome::Failed(ErrorClass::Timeout)),
            None,
            "still failed: no second line"
        );
        assert_eq!(h.failed(), [Source::Cpu]);
        assert!(
            h.record(Source::Cpu, Outcome::Ok)
                .unwrap()
                .contains("again")
        );
        assert!(h.failed().is_empty());
    }

    #[trace("FR-070-AC-1")]
    #[test]
    fn tc_070_01_the_default_profile_expects_every_source_but_temperature() {
        let p = Profile::default();
        for s in Source::ALL {
            assert_eq!(p.expects(s), s != Source::Temperature, "{}", s.name());
        }
        assert_eq!(Source::ALL.map(Source::name).len(), 9);
        assert!(Source::parse("pane_tail").is_some());
        assert!(Source::parse("gpu").is_none());
    }

    #[test]
    fn the_profile_comes_from_the_fleet_table_and_refuses_an_unknown_source() {
        let file: FleetFile = toml::from_str(
            "expected_sources = [\"cpu\", \"temperature\"]\nunavailable_sources = [\"compilers\"]",
        )
        .unwrap();
        let p = Profile::from_file(&file).unwrap();
        assert!(p.expects(Source::Temperature) && p.expects(Source::Cpu));
        assert!(!p.expects(Source::Memory));
        assert!(!p.expects(Source::Compilers));
        let bad: FleetFile = toml::from_str("expected_sources = [\"gpu\"]").unwrap();
        assert!(
            Profile::from_file(&bad)
                .unwrap_err()
                .contains("unknown source")
        );
        let bad: FleetFile = toml::from_str("expected_sources = 3").unwrap();
        assert!(Profile::from_file(&bad).is_err());
    }
}
