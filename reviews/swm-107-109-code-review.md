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
