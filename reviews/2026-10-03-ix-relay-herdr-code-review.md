---
id: SR-2065
title: "Code review — ix-relay Herdr features SWM-43 through SWM-46"
type: SpecReview
analysis: code-review
scope: "ix-relay@a7a0c3d1b4835d40ca683d816a5da67a1e3dc69b; crates/agent-msg/, crates/relay-core/, crates/relay-ops/, crates/relay-tui/, crates/relay-watcher/, plugins/herdr/, spec/functional/FR-049-herdr-relay-status.md, spec/functional/FR-050-herdr-online-lead-directory.md, spec/functional/FR-051-formatted-message-history-and-reply.md, spec/functional/FR-052-herdr-sidebar-relay-badge.md, spec/usecase/US-008-browse-leads-from-any-agent.md, spec/usecase/US-009-read-messages-safely.md, spec/tests.md, README.md"
review_set: subset
---

## Summary

Reviewed the SWM-43 through SWM-46 implementation at `a7a0c3d1b4835d40ca683d816a5da67a1e3dc69b`, including the Rust changes, Herdr manifest and docs, and the new requirements. The review found a watcher latency defect in sidebar reporting. The defect was fixed in `71d5065149e61d04b4a018be09993e0aa1eb94eb`, then the changed paths and review evidence were rerun.

## Verdict

**PASS WITH LIMITATIONS** — the identified delivery-loop defect is fixed and the relevant code checks pass. Live Herdr behavior and the full watcher suite still have verification limits listed below.

## Findings

| ID | Severity | Summary | Refs | Escape Cause |
| --- | --- | --- | --- | --- |
| FND-001 | high | When a sidebar report is due, `tick` synchronously calls `report_if_verified`, which calls `Herdr::get`; the real `get` may wait ten seconds. With several slow panes, the watcher serially delays delivery to every lead while performing display-only work. | `crates/relay-watcher/src/main.rs:156`, `crates/relay-watcher/src/sidebar.rs:37`, `crates/relay-watcher/src/herdr.rs:33` | implementation-bug-despite-evidence |

## Dispositions

| FND | Outcome | SHA / reason |
| --- | --- | --- |
| FND-001 | fixed | `71d5065149e61d04b4a018be09993e0aa1eb94eb`; reports run off the delivery loop, with at most four in flight and one report per pane. |

## Review Notes

- Herdr configuration and CLI references confirm that custom pane tokens use `$name`, token values have per-token TTL, and sidebar rows affect expanded desktop layout while collapsed/mobile layouts retain compact behavior: [Herdr configuration](https://herdr.dev/docs/configuration/), [Herdr CLI reference](https://herdr.dev/docs/cli-reference/).
- The review did not install or activate the plugin, and did not perform a live multi-agent send demonstration.
- Verification: watcher suite 301 passed and 3 failed on macOS process start-time lookup; directory suite 7 passed; `agent-msg` suite passed; workspace clippy, formatting, diff check, and test matrix passed. The three watcher failures are `native_identity_matches_claude_record_format`, `macos_native_process_identity_rejects_a_reused_pid`, and `the_start_time_is_read_past_a_command_name_with_parentheses` (current process start time unavailable).
- This does not establish that a newly launched Codex pane is immediately discoverable or that two live agents can exchange a reply through Herdr. No live Herdr installation or multi-agent send demonstration was run. FR-051-AC-4 and FR-052-AC-3/5 remain partial in `spec/tests.md`.
