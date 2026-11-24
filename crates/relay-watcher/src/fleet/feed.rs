//! The hook activity feed as the collector reads it (FR-068, FR-069, FR-070): each
//! lane's newest event of every kind, the windows that turn those into a lane state,
//! compiler attribution by hook-reported `cwd`, the ticket in a branch or `cwd`, and
//! the hook's local skip and drop counts.

use std::{collections::BTreeMap, fs, io, path::Path, time::Duration, time::Instant};

use relay_core::config::FleetFile;
use serde::Deserialize;

use super::{health::ErrorClass, lanes::HookSignal, pane::BlockedReason};

/// How long a lane's `cwd` and `branch` mapping lives without an event (FR-069).
pub const MAPPING_TTL: Duration = Duration::from_secs(6 * 60 * 60);

const MAX_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;

/// The configured windows of the lane state rules (FR-045 `ix.fleet.config`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Windows {
    /// How long a `prompt` or `post_tool` keeps a lane working.
    pub working_window: Duration,
    /// How long an unmatched `pre_tool` may run before the lane is stuck.
    pub max_tool_runtime: Duration,
    /// How long a non-idle lane may emit nothing before it is counted unhealthy.
    pub no_events_after: Duration,
}

impl Default for Windows {
    fn default() -> Self {
        Self {
            working_window: Duration::from_secs(10 * 60),
            max_tool_runtime: Duration::from_secs(2 * 60 * 60),
            no_events_after: Duration::from_secs(30 * 60),
        }
    }
}

impl Windows {
    /// The windows of the file's raw `[fleet]` values.
    ///
    /// # Errors
    ///
    /// A message naming the key when it is not a whole number of seconds from 1 to
    /// seven days.
    pub fn from_file(file: &FleetFile) -> Result<Self, String> {
        let mut windows = Self::default();
        for (key, raw, slot) in [
            (
                "working_window_secs",
                &file.working_window_secs,
                &mut windows.working_window,
            ),
            (
                "no_events_after_secs",
                &file.no_events_after_secs,
                &mut windows.no_events_after,
            ),
            (
                "max_tool_runtime_secs",
                &file.max_tool_runtime_secs,
                &mut windows.max_tool_runtime,
            ),
        ] {
            if let Some(raw) = raw {
                let secs = raw
                    .as_integer()
                    .and_then(|n| u64::try_from(n).ok())
                    .filter(|n| (1..=MAX_WINDOW_SECS).contains(n))
                    .ok_or_else(|| {
                        format!("[fleet] {key} must be a whole number of seconds from 1 to {MAX_WINDOW_SECS}")
                    })?;
                *slot = Duration::from_secs(secs);
            }
        }
        Ok(windows)
    }
}

/// The kinds of activity event, in the order a lane lives through them: among
/// events of the same millisecond the later one wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A session started.
    SessionStart,
    /// The user submitted a prompt.
    Prompt,
    /// A tool call is about to run.
    PreTool,
    /// A tool call finished.
    PostTool,
    /// The harness waits on a permission.
    Blocked,
    /// The harness reported an idle prompt.
    Idle,
    /// The turn ended.
    Stop,
    /// The session ended.
    SessionEnd,
}

const KINDS: [(Kind, &str); 8] = [
    (Kind::SessionStart, "session_start"),
    (Kind::Prompt, "prompt"),
    (Kind::PreTool, "pre_tool"),
    (Kind::PostTool, "post_tool"),
    (Kind::Blocked, "blocked"),
    (Kind::Idle, "idle"),
    (Kind::Stop, "stop"),
    (Kind::SessionEnd, "session_end"),
];

/// The part of a stored event the collector reads. The record is written by
/// another process: nothing else of it is used.
#[derive(Deserialize)]
struct Stored {
    server_ms: i64,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    last_class: Option<String>,
}

/// What the store holds about one lane: how long before the read each kind of
/// event last happened, and the newest event's `cwd` and `branch`.
#[derive(Debug, Clone)]
pub struct LaneActivity {
    read_at: Instant,
    ages_ms: [Option<u64>; 8],
    question: bool,
    cwd: String,
    branch: String,
}

impl LaneActivity {
    /// The activity of one lane from its `activity-last` record, read at `read_at`
    /// when the wall clock was `now_ms`. `None` when the record holds no event of
    /// a known kind.
    #[must_use]
    pub fn parse(record: &BTreeMap<String, String>, now_ms: i64, read_at: Instant) -> Option<Self> {
        let mut ages_ms = [None; 8];
        let mut newest: Option<(u64, Stored, Kind)> = None;
        for (index, (kind, name)) in KINDS.iter().enumerate() {
            let Some(stored) = record
                .get(&format!("kind:{name}"))
                .and_then(|raw| serde_json::from_str::<Stored>(raw).ok())
            else {
                continue;
            };
            // A clock ahead of the relay's reads as "just now".
            let age = u64::try_from(now_ms.saturating_sub(stored.server_ms)).unwrap_or(0);
            ages_ms[index] = Some(age);
            if newest.as_ref().is_none_or(|(a, _, _)| age <= *a) {
                newest = Some((age, stored, *kind));
            }
        }
        let (_, stored, kind) = newest?;
        Some(Self {
            read_at,
            ages_ms,
            question: kind == Kind::Stop && stored.last_class.as_deref() == Some("question"),
            cwd: stored.cwd,
            branch: stored.branch,
        })
    }

    fn age(&self, index: usize, now: Instant) -> Option<Duration> {
        self.ages_ms[index]
            .map(|ms| Duration::from_millis(ms) + now.saturating_duration_since(self.read_at))
    }

    /// The newest event's kind and age at `now`.
    #[must_use]
    pub fn newest(&self, now: Instant) -> Option<(Kind, Duration)> {
        let mut best: Option<(Kind, Duration)> = None;
        for (index, (kind, _)) in KINDS.iter().enumerate() {
            if let Some(age) = self.age(index, now)
                && best.is_none_or(|(_, a)| age <= a)
            {
                best = Some((*kind, age));
            }
        }
        best
    }

    /// The lane's newest hook evidence at `now` as a state verdict, and when it
    /// happened (`floor` when too long ago to represent).
    #[must_use]
    pub fn signal(
        &self,
        now: Instant,
        windows: &Windows,
        floor: Instant,
    ) -> Option<(HookSignal, Instant)> {
        let (kind, age) = self.newest(now)?;
        let at = now.checked_sub(age).unwrap_or(floor);
        let signal = match kind {
            Kind::SessionEnd => HookSignal::Ended,
            Kind::Blocked => HookSignal::Blocked(BlockedReason::Permission),
            Kind::Stop if self.question => HookSignal::Blocked(BlockedReason::Question),
            Kind::Stop | Kind::Idle | Kind::SessionStart => HookSignal::Idle,
            Kind::PreTool if age > windows.max_tool_runtime => HookSignal::Stuck,
            Kind::PreTool => HookSignal::Working,
            Kind::Prompt | Kind::PostTool if age <= windows.working_window => HookSignal::Working,
            Kind::Prompt | Kind::PostTool => HookSignal::Idle,
        };
        Some((signal, at))
    }

    /// The `cwd` of the newest event while its mapping lives (FR-069), else `None`.
    #[must_use]
    pub fn live_cwd(&self, now: Instant) -> Option<&str> {
        let (_, age) = self.newest(now)?;
        (age <= MAPPING_TTL && !self.cwd.is_empty()).then_some(self.cwd.as_str())
    }

    /// The newest event's branch.
    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// The newest event's `cwd`.
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }
}

/// Whether the lane's last known state is non-idle (FR-070): an open `pre_tool`,
/// a `prompt` with no later `stop`, or herdr working. A lane whose newest event
/// is `stop`, `idle` or `session_end` is quiet; one waiting on a permission is
/// waiting for a person, not failing to report.
#[must_use]
pub fn last_state_non_idle(
    activity: Option<&LaneActivity>,
    herdr_working: bool,
    now: Instant,
) -> bool {
    match activity.and_then(|a| a.newest(now)) {
        None => herdr_working,
        Some((Kind::PreTool | Kind::PostTool | Kind::Prompt, _)) => true,
        Some((Kind::SessionStart, _)) => herdr_working,
        Some((Kind::Blocked | Kind::Stop | Kind::Idle | Kind::SessionEnd, _)) => false,
    }
}

/// The lane whose `cwd` is the longest one containing `cwd`: an equal or an
/// enclosing `cwd` matches, a sibling that only shares a name prefix does not.
#[must_use]
pub fn attribute<'a>(cwd: &str, lanes: impl Iterator<Item = (usize, &'a str)>) -> Option<usize> {
    let cwd = Path::new(cwd);
    lanes
        .filter(|(_, lane_cwd)| !lane_cwd.is_empty() && cwd.starts_with(lane_cwd))
        .max_by_key(|(_, lane_cwd)| Path::new(lane_cwd).components().count())
        .map(|(index, _)| index)
}

/// The ticket key in a branch name, else in a working directory (`SWM-205` from
/// `swm-205-fleet-series` or `/w/ix-relay-swm-205`); empty when neither has one.
#[must_use]
pub fn ticket(branch: &str, cwd: &str) -> String {
    in_text(branch)
        .or_else(|| cwd.rsplit('/').find_map(in_text))
        .unwrap_or_default()
}

fn in_text(text: &str) -> Option<String> {
    for segment in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
        let parts: Vec<&str> = segment.split('-').collect();
        for pair in parts.windows(2) {
            let (team, number) = (pair[0], pair[1]);
            if (2..=10).contains(&team.len())
                && team.bytes().all(|b| b.is_ascii_alphabetic())
                && (1..=6).contains(&number.len())
                && number.bytes().all(|b| b.is_ascii_digit())
            {
                return Some(format!("{}-{number}", team.to_ascii_uppercase()));
            }
        }
    }
    None
}

/// The hook's local counts (`spool/activity.json`, written by `agent-msg`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct HookCounts {
    /// Events skipped by the backoff.
    #[serde(default)]
    pub skipped: u64,
    /// Events dropped because a write failed.
    #[serde(default)]
    pub dropped: u64,
    /// Events not written for want of a verified lane.
    #[serde(default)]
    pub unregistered: u64,
}

/// The counts in the file at `path`. A missing file is no hook activity lost, so
/// zero; an unreadable or unparsable one is an error, never zero.
///
/// # Errors
///
/// [`ErrorClass::Unreadable`] or [`ErrorClass::Parse`].
pub fn read_hook_counts(path: &Path) -> Result<HookCounts, ErrorClass> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| ErrorClass::Parse),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(HookCounts::default()),
        Err(_) => Err(ErrorClass::Unreadable),
    }
}

#[cfg(test)]
mod tests {
    use ix_trace_rs::trace;

    use super::*;

    #[trace("FR-045-AC-32")]
    #[test]
    fn windows_default_and_read_the_fleet_table_and_refuse_a_bad_value() {
        assert_eq!(Windows::default().working_window, Duration::from_secs(600));
        assert_eq!(
            Windows::default().max_tool_runtime,
            Duration::from_secs(7200)
        );
        let file: relay_core::config::FileConfig = toml::from_str(
            "[fleet]\nworking_window_secs = 60\nno_events_after_secs = 90\nmax_tool_runtime_secs = 120\n",
        )
        .unwrap();
        let w = Windows::from_file(&file.fleet).unwrap();
        assert_eq!(
            (w.working_window, w.no_events_after, w.max_tool_runtime),
            (
                Duration::from_secs(60),
                Duration::from_secs(90),
                Duration::from_secs(120)
            )
        );
        for bad in ["0", "-3", "\"x\"", "99999999"] {
            let file: relay_core::config::FileConfig =
                toml::from_str(&format!("[fleet]\nno_events_after_secs = {bad}\n")).unwrap();
            assert!(Windows::from_file(&file.fleet).is_err(), "{bad}");
        }
    }

    #[trace("FR-069-AC-5")]
    #[test]
    fn a_compiler_goes_to_the_longest_enclosing_cwd_or_to_none() {
        let lanes = ["/w/ix", "/w/ix/crates/a", "/w/other", "/w"];
        let pick = |cwd: &str| attribute(cwd, lanes.iter().copied().enumerate());
        assert_eq!(pick("/w/ix/crates/a/src"), Some(1), "nested wins");
        assert_eq!(pick("/w/ix/crates/a"), Some(1), "equal");
        assert_eq!(pick("/w/ix/crates/b"), Some(0), "inside");
        assert_eq!(
            pick("/w/ix-sibling"),
            Some(3),
            "a shared name prefix is not inside"
        );
        assert_eq!(pick("/elsewhere"), None, "outside");
        assert_eq!(attribute("/w/ix", [(0, "")].into_iter()), None, "no cwd");
    }

    #[trace("FR-045-AC-28")]
    #[test]
    fn the_ticket_comes_from_the_branch_then_the_cwd_and_is_empty_without_one() {
        assert_eq!(ticket("swm-205-fleet-series", "/x"), "SWM-205");
        assert_eq!(ticket("feature/QSL-12/more", ""), "QSL-12");
        assert_eq!(ticket("main", "/w/worktrees/ix-relay-swm-205"), "SWM-205");
        assert_eq!(ticket("main", "/w/ix-relay"), "");
        assert_eq!(ticket("", ""), "");
    }

    #[test]
    fn hook_counts_read_zero_for_a_missing_file_and_fail_for_a_broken_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.json");
        assert_eq!(read_hook_counts(&path), Ok(HookCounts::default()));
        fs::write(
            &path,
            r#"{"skipped":3,"dropped":2,"unregistered":1,"backoff_ms":5}"#,
        )
        .unwrap();
        assert_eq!(
            read_hook_counts(&path),
            Ok(HookCounts {
                skipped: 3,
                dropped: 2,
                unregistered: 1
            })
        );
        fs::write(&path, "{").unwrap();
        assert_eq!(read_hook_counts(&path), Err(ErrorClass::Parse));
    }
}
