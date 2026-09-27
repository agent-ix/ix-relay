//! `agent-msg hook <kind>`: the hook handler Claude Code and Codex call.
//!
//! Reads the harness's hook JSON on stdin, pops this lead's inbox, and prints the
//! hook output that puts the messages in front of the agent. Claude and Codex use
//! the same output shapes, so one handler serves both.

use std::num::NonZeroUsize;

use relay_core::{AddressBook, Message, SessionId};
use serde::{Deserialize, Serialize};

/// Most messages delivered in one hook call. The rest wait for the next one.
pub const HOOK_BATCH: NonZeroUsize = NonZeroUsize::new(50).expect("50 is non-zero");

/// Rendered-text budget for one hook call, in bytes. Both harnesses cap how
/// much of a hook's own output they keep: Codex keeps roughly the head and
/// tail of ~2500 tokens of `additionalContext` (see plugins/codex/README.md),
/// Claude Code caps hook output at roughly 10k characters. 6 KB sits well
/// under both, so a full batch never gets silently cut mid-message by the
/// harness itself — [`relay_core::Relay::pop_inbox_budget`] enforces it
/// against the size each message *renders to* (a line feed is 5 bytes printed
/// and 2 stored, and the sender label adds bytes), joined by one newline per gap.
/// Only the pending-count line comes on top.
pub const HOOK_BYTE_BUDGET: usize = 6 * 1024;

/// The fields we read from the harness's hook input.
///
/// No `deny_unknown_fields`: this payload belongs to the harness, which adds
/// fields between releases. Refusing them would break delivery on every upgrade.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    /// The harness session id.
    #[serde(default, deserialize_with = "lenient_session_id")]
    pub session_id: Option<SessionId>,
    /// The agent's working directory.
    pub cwd: Option<String>,
    /// Which hook fired.
    pub hook_event_name: String,
    /// Set when the call comes from inside a subagent. Subagents never read the
    /// lead's inbox; only the lead does.
    pub agent_id: Option<String>,
    /// Path to this session's transcript (Claude Code only; Codex omits
    /// it). Used by AGE-2018's name-precedence chain to read the last
    /// `/rename` title a lead's own session recorded — see
    /// `identity::custom_title_from_transcript`.
    pub transcript_path: Option<String>,
    /// `SessionStart`'s own `source` field: `"startup"`, `"resume"`,
    /// `"clear"`, or `"fork"` when Claude Code launched this session via
    /// `--fork-session` (verified live 2026-09-24). AGE-2018 PR review,
    /// finding 1: a fork of an investigating lead's own session must never
    /// join the bus under its own name, same as an `esc-*` tab — see
    /// `identity::derive_for_hook`.
    pub source: Option<String>,
}

/// A harness `session_id` that is text but not a [`SessionId`] (empty, longer
/// than a key should be, carrying a control character) reads as absent: the
/// hook never fails a turn over the payload (FR-017), and a session with no
/// usable id simply has no session cache. A value that is not text at all is
/// still a malformed payload, as before.
fn lenient_session_id<'de, D>(deserializer: D) -> Result<Option<SessionId>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.and_then(|s| s.parse().ok()))
}

/// The hook events we act on. Serialized under the harness's own event name,
/// e.g. `"PostToolUse"`, for the failure spool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// A session started or resumed: register, then deliver the backlog.
    SessionStart,
    /// Mid-turn, after a tool call.
    PostToolUse,
    /// The user just submitted a prompt.
    UserPromptSubmit,
    /// The turn is ending; block it to keep the agent working on new messages.
    Stop,
}

impl Event {
    /// Maps the harness's `hook_event_name`; `None` for events we ignore.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "SessionStart" => Some(Self::SessionStart),
            "PostToolUse" => Some(Self::PostToolUse),
            "UserPromptSubmit" => Some(Self::UserPromptSubmit),
            "Stop" => Some(Self::Stop),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::PostToolUse => "PostToolUse",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::Stop => "Stop",
        }
    }
}

/// What we print for the harness.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum HookOutput {
    /// Adds text to the agent's context.
    Context {
        /// The harness's per-event output block.
        #[serde(rename = "hookSpecificOutput")]
        specific: ContextOutput,
    },
    /// Stops the turn from ending and hands the agent `reason` as new input.
    Block {
        /// Always `"block"`.
        decision: &'static str,
        /// The messages.
        reason: String,
    },
}

/// The `hookSpecificOutput` block.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextOutput {
    hook_event_name: &'static str,
    additional_context: String,
}

/// The output for `event` carrying `messages`, or `None` when there is nothing to say.
///
/// `redelivered` says, for each message in turn, whether it is shown as a
/// redelivery (see [`relay_core::BudgetPopResult::redelivered`]); a missing entry
/// is `false`.
///
/// `pending` is how many more messages are still queued after this pop (see
/// [`relay_core::BudgetPopResult::pending`]); when it's nonzero, the text
/// always ends with a `(N more ix-relay messages pending)` line, so a batch
/// that didn't fit this call's byte budget is never silently lost — the next
/// hook call pops the rest.
pub fn respond(
    event: Event,
    messages: &[Message],
    redelivered: &[bool],
    pending: usize,
    book: &AddressBook,
) -> Option<HookOutput> {
    if messages.is_empty() {
        return None;
    }
    let mut text = messages
        .iter()
        .zip(redelivered.iter().copied().chain(std::iter::repeat(false)))
        .map(|(m, again)| book.render_marked(m, again))
        .collect::<Vec<_>>()
        .join("\n");
    if pending > 0 {
        text.push_str(&format!("\n({pending} more ix-relay messages pending)"));
    }
    Some(match event {
        Event::Stop => HookOutput::Block {
            decision: "block",
            reason: text,
        },
        Event::SessionStart | Event::PostToolUse | Event::UserPromptSubmit => HookOutput::Context {
            specific: ContextOutput {
                hook_event_name: event.name(),
                additional_context: text,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn msgs() -> Vec<Message> {
        ["first", "second"]
            .into_iter()
            .map(|text| Message {
                from: "qsl-lead".parse().unwrap(),
                to: "tl-lead".parse().unwrap(),
                text: text.into(),
                sent_at_ms: 0,
            })
            .collect()
    }

    #[test]
    fn context_events_emit_additional_context() {
        for (event, name) in [
            (Event::PostToolUse, "PostToolUse"),
            (Event::SessionStart, "SessionStart"),
            (Event::UserPromptSubmit, "UserPromptSubmit"),
        ] {
            let out = serde_json::to_value(
                respond(event, &msgs(), &[], 0, &AddressBook::default()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                out,
                json!({"hookSpecificOutput": {
                    "hookEventName": name,
                    "additionalContext":
                        "[ix-relay] from qsl-lead: first\n[ix-relay] from qsl-lead: second"
                }})
            );
        }
    }

    /// Tracing: TC-040-01
    /// ACs: FR-040-AC-1
    ///
    /// A redelivered message is marked in its signature and only that one; the
    /// stored message is untouched.
    #[test]
    fn a_redelivered_message_is_marked_and_a_new_one_is_not() {
        let out = respond(
            Event::PostToolUse,
            &msgs(),
            &[true, false],
            0,
            &AddressBook::default(),
        )
        .unwrap();
        let out = serde_json::to_value(out).unwrap();
        assert_eq!(
            out["hookSpecificOutput"]["additionalContext"],
            "[ix-relay] from qsl-lead (redelivered): first\n[ix-relay] from qsl-lead: second"
        );
    }

    #[test]
    fn the_sender_is_shown_by_address_when_the_book_knows_one() {
        let book = AddressBook::from_registrations(&[relay_core::Registration {
            name: "qsl-lead".parse().unwrap(),
            machine: "hub".parse().unwrap(),
            kind: "claude".into(),
            session_id: Some("session-1".parse().unwrap()),
            cwd: None,
            herdr_pane: Some("w1:p1".parse().unwrap()),
            place: Some(relay_core::Place {
                workspace: "EA".into(),
                tab: "1".into(),
                herdr_ids: Some(relay_core::HerdrPlaceIds {
                    workspace: "w1".parse().unwrap(),
                    tab: "w1:t1".parse().unwrap(),
                    pane: "w1:p1".parse().unwrap(),
                    session_id: "session-1".parse().unwrap(),
                    verified_at_ms: 1,
                }),
            }),
            last_seen_ms: 1,
        }]);
        let out =
            serde_json::to_value(respond(Event::Stop, &msgs(), &[], 0, &book).unwrap()).unwrap();
        assert_eq!(
            out,
            json!({"decision": "block",
                   "reason": "[ix-relay] from hub:EA:1: first\n[ix-relay] from hub:EA:1: second"})
        );
    }

    #[test]
    fn stop_blocks_with_messages_as_reason() {
        let out = serde_json::to_value(
            respond(Event::Stop, &msgs(), &[], 0, &AddressBook::default()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            out,
            json!({"decision": "block",
                   "reason": "[ix-relay] from qsl-lead: first\n[ix-relay] from qsl-lead: second"})
        );
    }

    #[test]
    fn empty_inbox_prints_nothing_so_stop_is_not_blocked() {
        assert!(respond(Event::Stop, &[], &[], 0, &AddressBook::default()).is_none());
        assert!(respond(Event::PostToolUse, &[], &[], 0, &AddressBook::default()).is_none());
    }

    #[test]
    fn nonzero_pending_appends_the_pending_count_line() {
        let out = serde_json::to_value(
            respond(Event::PostToolUse, &msgs(), &[], 3, &AddressBook::default()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            out,
            json!({"hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext":
                    "[ix-relay] from qsl-lead: first\n[ix-relay] from qsl-lead: second\n\
                     (3 more ix-relay messages pending)"
            }})
        );

        let blocked = serde_json::to_value(
            respond(Event::Stop, &msgs(), &[], 1, &AddressBook::default()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            blocked,
            json!({"decision": "block",
                   "reason": "[ix-relay] from qsl-lead: first\n[ix-relay] from qsl-lead: second\n\
                              (1 more ix-relay messages pending)"})
        );
    }

    #[test]
    fn zero_pending_appends_nothing() {
        let out = serde_json::to_value(
            respond(Event::PostToolUse, &msgs(), &[], 0, &AddressBook::default()).unwrap(),
        )
        .unwrap();
        let text = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext is a string");
        assert!(
            !text.contains("pending"),
            "zero pending must not add a pending line: {text:?}"
        );
    }

    #[test]
    fn parses_real_claude_hook_input() {
        // Captured from Claude Code on 2026-09-24 (AGE-2007 verify step).
        let raw = r#"{"session_id":"11111111-2222-4333-8444-555555555555",
            "transcript_path":"/x.jsonl","cwd":"/work","prompt_id":"p",
            "permission_mode":"default","hook_event_name":"Stop","stop_hook_active":false,
            "last_assistant_message":"OK","background_tasks":[],"session_crons":[]}"#;
        let input: HookInput = serde_json::from_str(raw).unwrap();
        assert_eq!(
            input.session_id.as_ref().map(|v| v.as_str()),
            Some("11111111-2222-4333-8444-555555555555")
        );
        assert_eq!(input.cwd.as_deref(), Some("/work"));
        assert_eq!(Event::parse(&input.hook_event_name), Some(Event::Stop));
        assert!(input.agent_id.is_none());
    }

    #[test]
    fn unknown_events_are_ignored() {
        assert_eq!(Event::parse("PreCompact"), None);
    }

    /// Claude Code's own `SessionStart` payload for a `--fork-session`
    /// launch (verified 2026-09-24): `source` is `"fork"`, distinct from
    /// `"startup"`/`"resume"`/`"clear"`. AGE-2018 PR review, finding 1.
    #[test]
    fn parses_a_fork_sessions_source_field() {
        let raw = r#"{"session_id":"11111111-2222-4333-8444-555555555555",
            "transcript_path":"/x.jsonl","cwd":"/work",
            "hook_event_name":"SessionStart","source":"fork"}"#;
        let input: HookInput = serde_json::from_str(raw).unwrap();
        assert_eq!(input.source.as_deref(), Some("fork"));
    }

    #[test]
    fn a_plain_startup_source_is_not_a_fork() {
        let raw = r#"{"session_id":"1","hook_event_name":"SessionStart","source":"startup"}"#;
        let input: HookInput = serde_json::from_str(raw).unwrap();
        assert_ne!(input.source.as_deref(), Some("fork"));
    }

    #[test]
    fn a_missing_source_field_is_none_not_an_error() {
        let raw = r#"{"session_id":"1","hook_event_name":"Stop"}"#;
        let input: HookInput = serde_json::from_str(raw).unwrap();
        assert_eq!(input.source, None);
    }

    /// Captured from a real headless `codex exec` session on 2026-09-24
    /// (AGE-2012 verify step), via a dump hook installed in an isolated
    /// CODEX_HOME: one real payload per event this hook acts on. Codex's hook
    /// input carries the same fields agent-msg reads (`session_id`, `cwd`,
    /// `hook_event_name`) under the same names as Claude Code, for every one.
    #[test]
    fn parses_real_codex_hook_input_for_every_event_we_act_on() {
        struct Case {
            event: Event,
            raw: &'static str,
        }
        let cases = [
            Case {
                event: Event::SessionStart,
                raw: r#"{"session_id":"01a0d6ac-ec6a-7ba0-9ae9-dd543ce5a184",
                    "transcript_path":"/x/rollout.jsonl","cwd":"/tmp/codex-probe-ws",
                    "hook_event_name":"SessionStart","model":"gpt-5.6-hub",
                    "permission_mode":"bypassPermissions","source":"startup"}"#,
            },
            Case {
                event: Event::PostToolUse,
                raw: r#"{"session_id":"01a0d6ac-ec6a-7ba0-9ae9-dd543ce5a184",
                    "turn_id":"01a0d6ac-eca2-7771-b3ca-ee22276f0f0c","transcript_path":"/x/rollout.jsonl",
                    "cwd":"/tmp/codex-probe-ws","hook_event_name":"PostToolUse","model":"gpt-5.6-hub",
                    "permission_mode":"bypassPermissions","tool_name":"Bash",
                    "tool_input":{"command":"echo step1"},"tool_response":"step1\n",
                    "tool_use_id":"exec-6b595672-7ecf-41a5-b00b-807edb79df35"}"#,
            },
            Case {
                event: Event::UserPromptSubmit,
                raw: r#"{"session_id":"01a0d6ac-ec6a-7ba0-9ae9-dd543ce5a184",
                    "turn_id":"01a0d6ac-eca2-7771-b3ca-ee22276f0f0c","transcript_path":"/x/rollout.jsonl",
                    "cwd":"/tmp/codex-probe-ws","hook_event_name":"UserPromptSubmit",
                    "model":"gpt-5.6-hub","permission_mode":"bypassPermissions",
                    "prompt":"Use the shell tool to run: echo step1"}"#,
            },
            Case {
                event: Event::Stop,
                raw: r#"{"session_id":"01a0d6ac-ec6a-7ba0-9ae9-dd543ce5a184",
                    "turn_id":"01a0d6ac-eca2-7771-b3ca-ee22276f0f0c","transcript_path":"/x/rollout.jsonl",
                    "cwd":"/tmp/codex-probe-ws","hook_event_name":"Stop","model":"gpt-5.6-hub",
                    "permission_mode":"bypassPermissions","stop_hook_active":false,
                    "last_assistant_message":"DONE"}"#,
            },
        ];
        for case in cases {
            let input: HookInput = serde_json::from_str(case.raw)
                .unwrap_or_else(|e| panic!("{:?} payload failed to parse: {e}", case.event));
            assert_eq!(
                input.session_id.as_ref().map(|v| v.as_str()),
                Some("01a0d6ac-ec6a-7ba0-9ae9-dd543ce5a184"),
                "{:?}: session_id",
                case.event
            );
            assert_eq!(
                input.cwd.as_deref(),
                Some("/tmp/codex-probe-ws"),
                "{:?}: cwd",
                case.event
            );
            assert_eq!(
                Event::parse(&input.hook_event_name),
                Some(case.event),
                "{:?}: hook_event_name",
                case.event
            );
            assert!(input.agent_id.is_none(), "{:?}: agent_id", case.event);
        }
    }

    /// A hook call made from *inside* a `spawn_agent` subagent, captured the same
    /// way. Codex marks these with a top-level `agent_id` (plus `agent_type`),
    /// exactly the field `HookInput` already reads to recognize Claude Code's own
    /// subagent marker — so the same `input.agent_id.is_some()` check in
    /// `hook_run::run` silences Codex subagents too, with no extra parsing needed.
    #[test]
    fn real_codex_subagent_hook_input_carries_agent_id() {
        let raw = r#"{"session_id":"01a0d6ac-ec6a-7ba0-9ae9-dd543ce5a184",
            "turn_id":"01a0d6ad-23a4-7fe1-996e-5c637a77c206",
            "agent_id":"01a0d6ad-2383-7220-ae2e-d97441f51665","agent_type":"default",
            "transcript_path":"/x/rollout.jsonl","cwd":"/tmp/codex-probe-ws",
            "hook_event_name":"PostToolUse","model":"gpt-5.6-hub",
            "permission_mode":"bypassPermissions","tool_name":"Bash",
            "tool_input":{"command":"echo subagent_ran"},"tool_response":"subagent_ran\n",
            "tool_use_id":"exec-25327c05-c735-4b7f-9f3e-5b28d4ca7e48"}"#;
        let input: HookInput = serde_json::from_str(raw).unwrap();
        // Same session_id as the lead (the root session), but agent_id is set:
        // this is what `hook_run::run` treats as "skip, this is a subagent".
        assert_eq!(
            input.session_id.as_ref().map(|v| v.as_str()),
            Some("01a0d6ac-ec6a-7ba0-9ae9-dd543ce5a184")
        );
        assert_eq!(
            input.agent_id.as_deref(),
            Some("01a0d6ad-2383-7220-ae2e-d97441f51665")
        );
    }

    /// Tracing: TC-008-06
    /// ACs: FR-008-AC-6
    ///
    /// A `session_id` that is text but no `SessionId` reads as absent (the hook
    /// never fails a turn over it); one that is not text is still malformed.
    #[test]
    fn an_unusable_session_id_reads_as_absent_and_a_non_string_is_malformed() {
        for raw in [
            r#"{"hook_event_name":"Stop","session_id":""}"#,
            r#"{"hook_event_name":"Stop","session_id":"a\u001bb"}"#,
            r#"{"hook_event_name":"Stop","session_id":null}"#,
            r#"{"hook_event_name":"Stop"}"#,
        ] {
            let input: HookInput = serde_json::from_str(raw).unwrap();
            assert_eq!(input.session_id, None, "{raw}");
        }
        let ok: HookInput =
            serde_json::from_str(r#"{"hook_event_name":"Stop","session_id":"s-1"}"#).unwrap();
        assert_eq!(ok.session_id.as_ref().map(SessionId::as_str), Some("s-1"));
        assert!(
            serde_json::from_str::<HookInput>(r#"{"hook_event_name":"Stop","session_id":7}"#)
                .is_err()
        );
    }
}
