//! Every Redis key the crate produces, each built by exactly one function here.
//!
//! No other code formats a key (the `keys_are_built_only_here` test greps for
//! it), and every builder that takes an id takes the typed id
//! ([`AgentName`], [`EscalationId`], [`SessionId`], [`MachineName`]), never a
//! `&str`: a raw string cannot reach a key. Adding a key means adding a builder
//! here and a row to `KEY_TABLE`, which the tests hold to the builders.

use crate::{
    escalation::EscalationId,
    ids::{MachineName, SessionId},
    name::AgentName,
};

/// The key namespace of one bus: the prefix plus one builder per key.
#[derive(Debug, Clone)]
pub(super) struct Keys {
    prefix: String,
}

impl Keys {
    pub(super) fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }

    pub(super) fn work_key(
        &self,
        machine: &MachineName,
        session: &SessionId,
        process_id: u32,
    ) -> String {
        format!("{}work:{machine}:{session}:{process_id}", self.prefix)
    }

    pub(super) fn inbox_key(&self, agent: &AgentName) -> String {
        format!("{}inbox:{agent}", self.prefix)
    }

    pub(super) fn agents_key(&self) -> String {
        format!("{}agents", self.prefix)
    }

    pub(super) fn aliases_key(&self) -> String {
        format!("{}aliases", self.prefix)
    }

    pub(super) fn log_key(&self) -> String {
        format!("{}log", self.prefix)
    }

    pub(super) fn health_hooks_key(&self) -> String {
        format!("{}health:hooks", self.prefix)
    }

    pub(super) fn esc_key(&self, id: &EscalationId) -> String {
        format!("{}esc:{id}", self.prefix)
    }

    pub(super) fn esc_attempts_key(&self, id: &EscalationId) -> String {
        format!("{}esc:{id}:attempts", self.prefix)
    }

    pub(super) fn esc_seq_key(&self) -> String {
        format!("{}esc:seq", self.prefix)
    }

    pub(super) fn esc_index_key(&self) -> String {
        format!("{}esc:index", self.prefix)
    }

    pub(super) fn session_key(&self, session_id: &SessionId) -> String {
        format!("{}session:{session_id}", self.prefix)
    }

    pub(super) fn suppressed_key(&self, session_id: &SessionId) -> String {
        format!("{}suppressed:{session_id}", self.prefix)
    }

    pub(super) fn unresolved_key(&self, session_id: &SessionId) -> String {
        format!("{}unresolved:{session_id}", self.prefix)
    }

    /// The once-per-window mark of a session whose herdr failure was recorded.
    pub(super) fn herdr_failed_key(&self, session_id: &SessionId) -> String {
        format!("{}herdr-failed:{session_id}", self.prefix)
    }

    pub(super) fn inflight_key(&self, agent: &AgentName) -> String {
        format!("{}inflight:{agent}", self.prefix)
    }

    /// What every [`Keys::inflight_key`] starts with, for the key scan.
    pub(super) fn inflight_stem(&self) -> String {
        format!("{}inflight:", self.prefix)
    }

    pub(super) fn dead_letter_key(&self, agent: &AgentName) -> String {
        format!("{}deadletter:{agent}", self.prefix)
    }

    /// What every [`Keys::dead_letter_key`] starts with, for the key scan.
    pub(super) fn dead_letter_stem(&self) -> String {
        format!("{}deadletter:", self.prefix)
    }

    pub(super) fn inflight_seq_key(&self) -> String {
        format!("{}inflight-seq", self.prefix)
    }

    pub(super) fn health_watcher_key(&self, machine: &MachineName) -> String {
        format!("{}health:watcher:{machine}", self.prefix)
    }

    /// What every [`Keys::health_watcher_key`] starts with, for the key scan.
    pub(super) fn health_watcher_stem(&self) -> String {
        format!("{}health:watcher:", self.prefix)
    }

    /// The non-expiring marker of each machine's last tick: a hash of machine to
    /// milliseconds, one field per machine that ever ran a heartbeat-writing
    /// watcher.
    pub(super) fn health_seen_key(&self) -> String {
        format!("{}health:watcher-seen", self.prefix)
    }

    /// The capped stream of relinks and renames (FR-043).
    pub(super) fn health_moves_key(&self) -> String {
        format!("{}health:moves", self.prefix)
    }

    /// One lead's [`crate::moves::PaneReplaced`] marker, a string with an expiry
    /// (FR-032).
    pub(super) fn health_replaced_key(&self, lead: &AgentName) -> String {
        format!("{}health:replaced:{lead}", self.prefix)
    }
}

#[cfg(test)]
impl Keys {
    /// A head under this namespace that no builder uses, for the scan tests.
    pub(super) fn scratch_head(&self, tail: &str) -> String {
        format!("{}{tail}", self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;

    /// Prefix used by the table; the real default would hide a builder that
    /// forgot `self.prefix`.
    const PREFIX: &str = "p:";

    /// `(builder name, the exact key it must produce)`: every key the crate can
    /// write, read or scan. A builder missing from here fails
    /// `every_builder_is_in_the_table`.
    fn key_table() -> Vec<(&'static str, String)> {
        let relay = Keys::new(PREFIX);
        let agent: AgentName = "lead-a".parse().unwrap();
        let id: EscalationId = "e1".parse().unwrap();
        let sid: SessionId = "sid-1".parse().unwrap();
        let machine: MachineName = "node-a".parse().unwrap();
        vec![
            ("work_key", relay.work_key(&machine, &sid, 42)),
            ("inbox_key", relay.inbox_key(&agent)),
            ("agents_key", relay.agents_key()),
            ("aliases_key", relay.aliases_key()),
            ("log_key", relay.log_key()),
            ("health_hooks_key", relay.health_hooks_key()),
            ("esc_key", relay.esc_key(&id)),
            ("esc_attempts_key", relay.esc_attempts_key(&id)),
            ("esc_seq_key", relay.esc_seq_key()),
            ("esc_index_key", relay.esc_index_key()),
            ("session_key", relay.session_key(&sid)),
            ("suppressed_key", relay.suppressed_key(&sid)),
            ("unresolved_key", relay.unresolved_key(&sid)),
            ("herdr_failed_key", relay.herdr_failed_key(&sid)),
            ("inflight_key", relay.inflight_key(&agent)),
            ("inflight_stem", relay.inflight_stem()),
            ("dead_letter_key", relay.dead_letter_key(&agent)),
            ("dead_letter_stem", relay.dead_letter_stem()),
            ("inflight_seq_key", relay.inflight_seq_key()),
            ("health_watcher_key", relay.health_watcher_key(&machine)),
            ("health_watcher_stem", relay.health_watcher_stem()),
            ("health_seen_key", relay.health_seen_key()),
            ("health_moves_key", relay.health_moves_key()),
            ("health_replaced_key", relay.health_replaced_key(&agent)),
        ]
    }

    #[test]
    fn the_keys_are_the_documented_ones() {
        let want = [
            ("work_key", "p:work:node-a:sid-1:42"),
            ("inbox_key", "p:inbox:lead-a"),
            ("agents_key", "p:agents"),
            ("aliases_key", "p:aliases"),
            ("log_key", "p:log"),
            ("health_hooks_key", "p:health:hooks"),
            ("esc_key", "p:esc:e1"),
            ("esc_attempts_key", "p:esc:e1:attempts"),
            ("esc_seq_key", "p:esc:seq"),
            ("esc_index_key", "p:esc:index"),
            ("session_key", "p:session:sid-1"),
            ("suppressed_key", "p:suppressed:sid-1"),
            ("unresolved_key", "p:unresolved:sid-1"),
            ("herdr_failed_key", "p:herdr-failed:sid-1"),
            ("inflight_key", "p:inflight:lead-a"),
            ("inflight_stem", "p:inflight:"),
            ("dead_letter_key", "p:deadletter:lead-a"),
            ("dead_letter_stem", "p:deadletter:"),
            ("inflight_seq_key", "p:inflight-seq"),
            ("health_watcher_key", "p:health:watcher:node-a"),
            ("health_watcher_stem", "p:health:watcher:"),
            ("health_seen_key", "p:health:watcher-seen"),
            ("health_moves_key", "p:health:moves"),
            ("health_replaced_key", "p:health:replaced:lead-a"),
        ];
        let got = key_table();
        assert_eq!(got.len(), want.len());
        for ((name, key), (want_name, want_key)) in got.iter().zip(want) {
            assert_eq!((name.to_owned(), key.as_str()), (want_name, want_key));
        }
    }

    /// Names of the functions this file defines that build a key.
    fn builders_in_source() -> Vec<String> {
        let src = include_str!("keys.rs");
        let code = src.split("#[cfg(test)]").next().unwrap_or(src);
        code.lines()
            .filter_map(|l| l.trim().strip_prefix("pub(super) fn "))
            .filter_map(|l| l.split('(').next())
            .filter(|name| *name != "new")
            .map(str::to_owned)
            .collect()
    }

    /// Every builder under another prefix: the prefix is the only thing that
    /// changes, so no builder hard-codes one or drops it.
    #[test]
    fn every_key_carries_its_prefix_and_nothing_else_changes() {
        let default: Vec<(&str, String)> = key_table();
        let other = Keys::new("other-prefix|");
        let agent: AgentName = "lead-a".parse().unwrap();
        let id: EscalationId = "e1".parse().unwrap();
        let sid: SessionId = "sid-1".parse().unwrap();
        let machine: MachineName = "node-a".parse().unwrap();
        let moved = [
            other.work_key(&machine, &sid, 42),
            other.inbox_key(&agent),
            other.agents_key(),
            other.aliases_key(),
            other.log_key(),
            other.health_hooks_key(),
            other.esc_key(&id),
            other.esc_attempts_key(&id),
            other.esc_seq_key(),
            other.esc_index_key(),
            other.session_key(&sid),
            other.suppressed_key(&sid),
            other.unresolved_key(&sid),
            other.herdr_failed_key(&sid),
            other.inflight_key(&agent),
            other.inflight_stem(),
            other.dead_letter_key(&agent),
            other.dead_letter_stem(),
            other.inflight_seq_key(),
            other.health_watcher_key(&machine),
            other.health_watcher_stem(),
            other.health_seen_key(),
            other.health_moves_key(),
            other.health_replaced_key(&agent),
        ];
        assert_eq!(default.len(), moved.len());
        for ((name, key), moved) in default.iter().zip(&moved) {
            assert_eq!(
                Some(moved.as_str()),
                key.strip_prefix(PREFIX)
                    .map(|tail| format!("other-prefix|{tail}"))
                    .as_deref(),
                "{name}"
            );
        }
    }

    #[test]
    fn every_builder_is_in_the_table() {
        let mut in_source = builders_in_source();
        let mut in_table: Vec<String> =
            key_table().into_iter().map(|(n, _)| n.to_owned()).collect();
        in_source.sort();
        in_table.sort();
        assert_eq!(in_source, in_table, "a key builder and KEY_TABLE disagree");
    }

    /// Rust files of this crate's `src`, with the test module stripped.
    fn production_sources(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                production_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if name.ends_with("_tests.rs") || name == "keys.rs" || name == "test_env.rs" {
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap();
                let code = text
                    .split("#[cfg(test)]\nmod tests")
                    .next()
                    .unwrap_or(&text)
                    .to_owned();
                out.push((path.display().to_string(), code));
            }
        }
    }

    /// Whether `line` calls a method on the result of a `keys.<builder>(..)` call.
    fn chained_on_a_key(line: &str) -> bool {
        let mut rest = line;
        while let Some(at) = rest.find("self.keys.") {
            rest = &rest[at + "self.keys.".len()..];
            let Some(open) = rest.find('(') else { break };
            let Some(close) = rest[open..].find(')') else {
                break;
            };
            let after = &rest[open + close + 1..];
            if after.starts_with('.') && !after.starts_with(".arg(") {
                return true;
            }
        }
        false
    }

    /// The guard itself: a mutant that derives a key from another builder is seen.
    #[test]
    fn the_guard_sees_a_key_derived_from_another_builder() {
        assert!(chained_on_a_key(
            r#"let k = self.keys.log_key().trim_end_matches("log");"#
        ));
        assert!(!chained_on_a_key(
            "pipe.cmd(\"GET\").arg(self.keys.agents_key())"
        ));
        assert!(!chained_on_a_key("let key = self.keys.agents_key();"));
    }

    #[test]
    fn keys_are_built_only_here() {
        let mut sources = Vec::new();
        // `src/store.rs` and everything under `src/store/`: the only code with a
        // connection, hence the only code that could build a key.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        production_sources(&src.join("store"), &mut sources);
        let store = src.join("store.rs");
        sources.push((
            store.display().to_string(),
            fs::read_to_string(&store)
                .unwrap()
                .split("#[cfg(test)]\nmod tests")
                .next()
                .unwrap_or_default()
                .to_owned(),
        ));
        assert!(sources.len() > 4, "the guard found no sources to check");
        // The only mentions of the default prefix outside this file: its
        // definition, and the `Relay` constructor handing it to `Keys`.
        const ALLOWED: [&str; 2] = ["pub const DEFAULT_PREFIX", "Keys::new(DEFAULT_PREFIX)"];
        for (path, code) in sources {
            for (n, line) in code.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                let raw_key = !ALLOWED.iter().any(|a| line.contains(a))
                    && (line.contains(".prefix")
                        || line.contains("\"ixr:")
                        || line.contains("DEFAULT_PREFIX"));
                // A key is used whole. Deriving one from another builder's output
                // (`log_key().trim_end_matches("log")`, `format!("{}x", keys.log_key())`)
                // is a key formatted outside this file too.
                let derived =
                    line.contains("keys.") && (line.contains("format!") || chained_on_a_key(line));
                assert!(
                    !raw_key && !derived,
                    "{path}:{}: a Redis key is built outside keys.rs: {line}",
                    n + 1
                );
            }
        }
    }
}
