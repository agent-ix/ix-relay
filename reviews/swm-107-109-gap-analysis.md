---
id: SR-070
title: "Gap analysis — ix-relay SWM-107–109"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/, crates/agent-msg/, crates/relay-core/, tests/"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/spec", type: references }
---

## Summary

Reviewed PR #26 at `99ec6afe7bfa6ae578bf73f56ac6611ed8f3dc80`. FR-056 through FR-058 now own the status request/reply, explicit lane focus, and explicit server-capacity behavior. Their acceptance criteria are traced to the implementation and tests. No new specification gap was found.

## Verdict

**PASS** — the in-scope behavior has owning requirements and no open gap-analysis findings.

## Scope

| Unit | Role | Excerpt |
| --- | --- | --- |
| FR-056-AC-1–7 | examined | Fixed-form status requests and replies, numeric spare coder slots, collection, and safe treatment of reply text. |
| FR-057-AC-1–7 | examined | Explicit lane focus, set/clear isolation, unset state, and stale marking. |
| FR-058-AC-1–6 | examined | Operator-entered server capacity and grouped lane display without inferred values. |

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | The initial implementation had no owning requirement for status request/reply behavior. | crates/agent-msg/src/main.rs::Cmd::StatusRequest, crates/agent-msg/src/status_request.rs::request, crates/relay-core/src/status.rs::StatusReply |
| FND-002 | high | The initial implementation had no owning requirement for focus set/clear and stale display. | crates/agent-msg/src/main.rs::Cmd::Focus, crates/agent-msg/src/status_request.rs::focus_list |
| FND-003 | high | The initial implementation had no owning requirement for capacity set/clear and grouped display. | crates/agent-msg/src/main.rs::Cmd::ServerCapacity, crates/agent-msg/src/status_request.rs::server_list |

## Dispositions

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | FR-056 defines request/reply fields, required numeric `spare_coder_slots`, collection behavior, and untrusted reply text handling. |
| FND-002 | fixed | FR-057 defines explicit focus set/clear, unset state, and stale-age behavior. |
| FND-003 | fixed | FR-058 defines operator-entered capacity and grouped server output with no inferred capacity. |

## Coverage

- Requirements: FR-056 through FR-058 in `spec/functional/` own the changed user-visible behaviors.
- Traceability: the command integration tests in `crates/agent-msg/tests/program_review.rs` carry criterion trace tags; core unit tests cover validation and stale-boundary behavior.
- Redis-backed runtime evidence: the tests require `IX_RELAY_TEST_URL`; this review did not independently run them.

## Exact-head reconciliation (`99ec6afe7bfa6ae578bf73f56ac6611ed8f3dc80`)

The former proposal to serialize an omitted spare-coder-slot value as JSON `null` is superseded. Merged FR-056 requires `--spare-coder-slots` and defines `spare_coder_slots` as an integer from 0 through 65535. The implementation requires the flag, stores the numeric value, and emits it numerically. The latest code also requires `submitted_at_ms` on the wire, while assigning its value on receipt, and no longer enforces owner-rule count/length limits absent from FR-058. The criterion trace additions link status request notice semantics, untrusted owner-decision text, and capacity-only servers to their tests. No new specification gaps were found.
