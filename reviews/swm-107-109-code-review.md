---
id: SR-069
title: "Code review — ix-relay SWM-107–109"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/agent-msg/src/main.rs, crates/agent-msg/src/status_request.rs, crates/relay-core/src/status.rs, crates/relay-core/src/lane_registry.rs, crates/relay-core/src/store.rs, crates/relay-core/src/store/status.rs, crates/relay-core/src/store/keys.rs, crates/relay-core/src/lib.rs"
review_set: subset
---

## Summary

Reviewed PR #26 at `e35e6b9835ad2baad8494b0296cb6f2b4f0d0601` for the SWM-107–109 status request, focus, and server capacity changes. The Rust source passes tests and formatting; the lint gate fails on a newly added test, and the new CLI behavior has no direct CLI-level acceptance tests.

## Verdict

**CONDITIONAL** — two medium findings remain: strict Clippy fails on a new test, and the newly exposed CLI contracts are not directly exercised by tests.

## Scope

| Unit | Role | Excerpt |
| --- | --- | --- |
| SWM-107-AC-1 | examined | Request and replies carry a request id; redelivery does not duplicate a reply |
| SWM-107-AC-2 | examined | JSON output for the collected replies |
| SWM-107-AC-3 | examined | A lead can answer by hand or through team-minder (separate ticket) |
| SWM-107-AC-4 | examined | Tests for send, partial collection and redelivery |
| SWM-108-AC-1 | examined | A lead sets and clears its focus with one command |
| SWM-108-AC-2 | examined | `who --focus` shows focus and when it was last set |
| SWM-108-AC-3 | examined | Stale focus (older than a configurable age) is marked, not hidden |
| SWM-109-AC-1 | examined | Capacity is set by the operator, not guessed |
| SWM-109-AC-2 | examined | Shows which lanes run on each server |

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | A new test fails the repository's Clippy `-D warnings` gate | crates/relay-core/src/status.rs:283 |
| FND-002 | medium | Tests exercise relay storage but do not invoke the new CLI commands or verify their output contracts | crates/agent-msg/src/status_request.rs:42-200 |

## Review notes

- `cargo fmt --check`: passed (stable rustfmt emitted warnings that configured import options require nightly).
- `cargo test -p relay-core -p agent-msg`: passed (189 agent-msg tests; relay-core unit and integration suites passed; Redis-backed cases return early because `IX_RELAY_TEST_URL` is unset).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: failed at `status.rs:283`; `&[requester.clone()]` triggers `clippy::cloned_ref_to_slice_refs`.
- `cargo deny check`: exited 0; it reported existing workspace dependency and yanked crate warnings.
- GitHub CI workflow files are unchanged. PR body already references SWM-107, so no PR comment was needed.
- Redis-backed request/redelivery and focus/capacity persistence behavior could not be run against Redis in this environment.

## Current-head follow-up (`1b21e665673f0213469713a485b01634b597b9ef`)

Reviewed the added unknown-slot behavior and rechecked the prior findings. `StatusReply.spare_coder_slots` is now `Option<u16>`; omitting the CLI flag stores `None`, which serde emits as JSON `null`, while an explicit count remains numeric. `tc_056_status_commands_parse_arguments_and_report_the_collection_contract` asserts both an explicit value (`2`) and an omitted value (`null`), and FR-056-AC-4 specifies that contract. The prior Clippy and CLI-coverage findings remain fixed. No new findings.


## Dispositions

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | Commit d34cf353 replaces the cloned one-item slice with `std::slice::from_ref`. |
| FND-002 | fixed | Commit d34cf353 adds direct CLI integration tests for status request/reply collection, focus, and server capacity. The tests were skipped at runtime here because Redis is unavailable. |

After excerpts, verified in commit d34cf353:

- FND-001: `assert!(validate_request(&requester, std::slice::from_ref(&requester)).is_err());`
- FND-002: `fn tc_056_status_commands_parse_arguments_and_report_the_collection_contract()` invokes `status-request`, `status-reply`, and `status-replies`; `fn tc_057_focus_commands_set_clear_and_render_stale_focus()` invokes `focus` and `who --focus`; `fn tc_058_capacity_commands_record_clear_and_group_server_lanes()` invokes `server-capacity` and `who --servers`.


### Round 2

| FND | outcome | reason |
| --- | --- | --- |
| FND-003 | fixed | At `ab53e1141c1c9ea634eabf020e0d64b29b692919`, `tc_056_status_commands_parse_arguments_and_report_the_collection_contract` now exercises all and explicit selection, unknown/duplicate/self requests, an unrequested responder, partial collection JSON with a nonresponder, and changed-payload first-write-wins. Redis-backed execution is not independently confirmed in this environment: `make test-redis` could not fetch the pinned ix-board dependency because GitHub authentication failed. |

After excerpt, verified at `crates/agent-msg/tests/program_review.rs` in `ab53e11`:

- FND-003: `for target in ["hub:EA:missing", "hub:EA:code,hub:EA:code", "hub:EA:sender"] {` followed by nonzero and nonempty-stderr assertions; separate assertions cover `all`, unrequested reply rejection, partial JSON `replies=[]` plus the requested lane in `nonresponders`, and changed-payload duplicate reply preserving `SWM-107` / `active`.

## Disposition pass 1 scope additions

| Unit | Role | Excerpt |
| --- | --- | --- |
| FR-056-AC-1 | examined | Given `all` or explicit registered addresses, `status-request` creates a request id, reports the selected lanes, and reports each enqueue outcome; unknown, duplicate, and self lanes are refused. |
| FR-056-AC-2 | examined | Given a request id, a selected lane can store one valid fixed-form reply; an unrequested lane is refused and a later reply cannot replace the first. |
| FR-056-AC-3 | examined | Given a partly answered request, `status-replies` emits JSON with the request id, requested lanes, replies, and nonresponders. |
| FR-057-AC-1 | examined | Given a registered lead, `focus <text>` stores that text and `focus --clear` removes it. |
| FR-057-AC-2 | examined | Given registered lanes with set and unset focus, `who --focus` displays each lane, its focus or unset state, and the set time. |
| FR-057-AC-3 | examined | Given a focus older than the configured stale age, `who --focus --stale-after` marks it `STALE` and still prints the focus. |
| FR-058-AC-1 | examined | Given valid capacity values, `server-capacity set` records them and reports the server; `clear` removes the record. |
| FR-058-AC-2 | examined | Given registered lanes on multiple servers, `who --servers` groups their names by server and displays the explicit capacity or unset state. |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-003 | medium | FR-056's new CLI test does not exercise the acceptance criteria's rejection or partial-collection cases | crates/agent-msg/tests/program_review.rs:45-106 |

The test covers one explicitly selected lane, a valid reply, a repeated identical reply, and a fully answered collection. It does not test `all`, unknown/duplicate/self-targeted lanes, an unrequested responder, a changed second reply, or a partly answered CLI collection with a nonresponder. Those acceptance branches can regress while the newly tagged tests stay green.

## Current-head reconciliation

The follow-up proposal to allow an unknown spare-coder-slot count to serialize
as JSON `null` is not included. The merged FR-056 contract requires
`--spare-coder-slots` and defines the field as an integer. The implementation
and tests retain the required argument and numeric JSON value.
