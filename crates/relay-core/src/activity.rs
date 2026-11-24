// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Hook activity events (FR-068): one compact record per harness hook call.
//!
//! The event carries no tool input, tool output, prompt or message text: only
//! names, classes and bounded strings. [`ActivityEvent::encode`] applies every
//! bound, so a caller cannot write an oversized or control-character event.

use std::collections::BTreeMap;

use serde::Serialize;

/// Most events the activity stream keeps.
pub const ACTIVITY_MAX_EVENTS: usize = 20_000;
/// Oldest event the activity stream keeps, in milliseconds.
pub const ACTIVITY_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;
/// How long a lane's last-activity record outlives its newest write, in seconds.
pub const ACTIVITY_LAST_TTL_SECS: u64 = 7 * 24 * 60 * 60;
/// Longest string field, in bytes.
pub const ACTIVITY_MAX_FIELD_BYTES: usize = 256;
/// Longest `cwd`, in bytes.
pub const ACTIVITY_MAX_CWD_BYTES: usize = 1024;
/// Longest stored event, in bytes.
pub const ACTIVITY_MAX_EVENT_BYTES: usize = 4096;
/// Most skill entries in a `session_start` event.
pub const ACTIVITY_MAX_SKILL_PAIRS: usize = 16;
/// Longest skill or role-spec name, in bytes.
pub const ACTIVITY_MAX_SKILL_NAME_BYTES: usize = 64;
/// Longest version, in bytes.
pub const ACTIVITY_MAX_SKILL_VERSION_BYTES: usize = 32;

/// What the relay adds to the encoded event on write: the lane address and the
/// server time. The encoded body stays under the cap by this much.
const WRITE_OVERHEAD_BYTES: usize = ACTIVITY_MAX_FIELD_BYTES + 64;

/// The kind of one activity event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    /// A tool call is about to run.
    PreTool,
    /// A tool call finished.
    PostTool,
    /// The user submitted a prompt.
    Prompt,
    /// The harness is waiting on the user for a permission.
    Blocked,
    /// The harness reported an idle prompt.
    Idle,
    /// The turn ended.
    Stop,
    /// A session started.
    SessionStart,
    /// A session ended.
    SessionEnd,
}

impl ActivityKind {
    /// The kind as written in the event.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreTool => "pre_tool",
            Self::PostTool => "post_tool",
            Self::Prompt => "prompt",
            Self::Blocked => "blocked",
            Self::Idle => "idle",
            Self::Stop => "stop",
            Self::SessionStart => "session_start",
            Self::SessionEnd => "session_end",
        }
    }
}

/// Which harness wrote the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
}

/// What a `Stop` event says about the final assistant message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LastClass {
    /// The message asks the user something.
    Question,
    /// Any other available message.
    Done,
    /// No message was available.
    Unknown,
}

/// Why a `blocked` event blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    /// A permission prompt.
    Permission,
}

/// One hook activity event, before the relay adds the lane address and server time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivityEvent {
    /// The harness session id.
    pub session: String,
    /// Unix milliseconds on the writing host.
    pub ts: i64,
    /// The kind.
    pub kind: ActivityKind,
    /// The working directory.
    pub cwd: String,
    /// The checked-out branch of `cwd`, or empty.
    pub branch: String,
    /// The harness.
    pub harness: Harness,
    /// The tool name, for `pre_tool` and `post_tool`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The reason, for `blocked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<BlockedReason>,
    /// The final-message class, for `stop`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_class: Option<LastClass>,
    /// The role spec, for `session_start`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Skill name to version, for `session_start`, where it is always present
    /// (an empty map when no version is known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_versions: Option<BTreeMap<String, String>>,
}

impl ActivityEvent {
    /// An event of `kind` with nothing kind-specific set.
    #[must_use]
    pub fn new(kind: ActivityKind, harness: Harness, session: &str, ts: i64) -> Self {
        Self {
            session: session.to_owned(),
            ts,
            kind,
            cwd: String::new(),
            branch: String::new(),
            harness,
            tool: None,
            reason: None,
            last_class: None,
            role: None,
            skill_versions: None,
        }
    }

    /// The event as the JSON object the relay stores, every bound applied.
    ///
    /// Strings are cut on a character boundary after control characters are
    /// removed. The result leaves room for the `addr` and `server_ms` fields
    /// the relay adds, so the stored event is at most
    /// [`ACTIVITY_MAX_EVENT_BYTES`]; skill pairs are dropped from the end
    /// until it fits.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut bounded = Self {
            session: bound(&self.session, ACTIVITY_MAX_FIELD_BYTES),
            ts: self.ts,
            kind: self.kind,
            cwd: bound(&self.cwd, ACTIVITY_MAX_CWD_BYTES),
            branch: bound(&self.branch, ACTIVITY_MAX_FIELD_BYTES),
            harness: self.harness,
            tool: self
                .tool
                .as_ref()
                .map(|v| bound(v, ACTIVITY_MAX_FIELD_BYTES)),
            reason: self.reason,
            last_class: self.last_class,
            role: self
                .role
                .as_ref()
                .map(|v| bound(v, ACTIVITY_MAX_FIELD_BYTES)),
            skill_versions: self.skill_versions.as_ref().map(|versions| {
                versions
                    .iter()
                    .take(ACTIVITY_MAX_SKILL_PAIRS)
                    .map(|(name, version)| bound_pair(name, version))
                    .collect()
            }),
        };
        loop {
            let json = serde_json::to_string(&bounded).unwrap_or_default();
            if json.len() + WRITE_OVERHEAD_BYTES <= ACTIVITY_MAX_EVENT_BYTES
                || bounded
                    .skill_versions
                    .as_mut()
                    .and_then(BTreeMap::pop_last)
                    .is_none()
            {
                return json;
            }
        }
    }
}

/// A skill name and version, each cut to its bound; an empty version is
/// `unknown`.
fn bound_pair(name: &str, version: &str) -> (String, String) {
    let version = if version.trim().is_empty() {
        "unknown"
    } else {
        version
    };
    (
        bound(name, ACTIVITY_MAX_SKILL_NAME_BYTES),
        bound(version, ACTIVITY_MAX_SKILL_VERSION_BYTES),
    )
}

/// `text` without control characters, cut to at most `max` bytes on a
/// character boundary.
fn bound(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use ix_trace_rs::trace;

    use super::*;

    fn event() -> ActivityEvent {
        ActivityEvent::new(ActivityKind::PostTool, Harness::Claude, "s-1", 5)
    }

    #[trace("FR-068-AC-6")]
    #[test]
    fn an_oversized_field_is_cut_clean_and_the_event_stays_under_four_kib() {
        let mut e = event();
        e.tool = Some(format!("{}\u{7}{}", "a".repeat(255), "é".repeat(5_000)));
        e.cwd = "x".repeat(10_240);
        e.branch = "b".repeat(10_240);
        e.session = "s".repeat(10_240);
        let json = e.encode();
        assert!(
            json.len() + 64 + 256 <= ACTIVITY_MAX_EVENT_BYTES,
            "{}",
            json.len()
        );
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let tool = v["tool"].as_str().unwrap();
        assert!(tool.len() <= ACTIVITY_MAX_FIELD_BYTES);
        assert!(!tool.contains('\u{7}'));
        assert!(tool.starts_with(&"a".repeat(255)));
        // The cut fell inside a two-byte character; none was kept half.
        assert_eq!(tool.len(), 255);
        assert_eq!(v["cwd"].as_str().unwrap().len(), ACTIVITY_MAX_CWD_BYTES);
        assert_eq!(
            v["branch"].as_str().unwrap().len(),
            ACTIVITY_MAX_FIELD_BYTES
        );
    }

    #[trace("FR-068-AC-5")]
    #[test]
    fn skill_pairs_are_capped_bounded_and_never_without_a_version() {
        let mut e = ActivityEvent::new(ActivityKind::SessionStart, Harness::Codex, "s-1", 5);
        let mut versions: BTreeMap<String, String> = (0..20)
            .map(|i| (format!("skill-{i:02}"), format!("1.{i}")))
            .collect();
        versions.insert("n".repeat(200), "9.9".into());
        versions.insert("bare".into(), String::new());
        versions.insert("v".into(), format!("1.{}", "9".repeat(100)));
        e.skill_versions = Some(versions);
        let v: serde_json::Value = serde_json::from_str(&e.encode()).unwrap();
        let map = v["skill_versions"].as_object().unwrap();
        assert_eq!(map.len(), ACTIVITY_MAX_SKILL_PAIRS);
        for (name, version) in map {
            assert!(name.len() <= 64 && version.as_str().unwrap().len() <= 32);
        }
        let mut one = ActivityEvent::new(ActivityKind::SessionStart, Harness::Codex, "s-1", 5);
        one.skill_versions = Some(BTreeMap::from([("bare".to_owned(), String::new())]));
        let v: serde_json::Value = serde_json::from_str(&one.encode()).unwrap();
        assert_eq!(v["skill_versions"]["bare"], "unknown");
        one.skill_versions = Some(BTreeMap::new());
        let v: serde_json::Value = serde_json::from_str(&one.encode()).unwrap();
        assert_eq!(v["skill_versions"], serde_json::json!({}));
    }

    #[trace("FR-068-AC-1")]
    #[test]
    fn an_event_without_a_tool_input_field_encodes_only_its_schema() {
        let mut e = event();
        e.tool = Some("Bash".into());
        let v: serde_json::Value = serde_json::from_str(&e.encode()).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["branch", "cwd", "harness", "kind", "session", "tool", "ts"]
        );
        assert_eq!(v["kind"], "post_tool");
        assert_eq!(v["harness"], "claude");
    }
}
