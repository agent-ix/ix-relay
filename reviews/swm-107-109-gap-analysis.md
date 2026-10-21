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

Repository-driven gap analysis for the SWM-107–109 implementation. The new status request, lane focus, and server capacity interfaces are user-visible behavior with no owning requirement in `spec/`; the repository's computed matrix also reports 345 of 358 criteria as untagged, a pre-existing repository-wide condition.

## Verdict

**FAIL** — newly added CLI and relay behavior has no owning specification requirement.

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
| FR-056-AC-4 | examined | An omitted spare-coder-slot count is emitted as JSON `null`, distinct from zero |

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | SWM-107 status request/reply commands and persisted response behavior have no owning requirement in `spec/` | crates/agent-msg/src/main.rs::Cmd::StatusRequest, crates/agent-msg/src/status_request.rs::request, crates/relay-core/src/status.rs::StatusReply |
| FND-002 | high | SWM-108 focus set/clear and stale focus display have no owning requirement in `spec/` | crates/agent-msg/src/main.rs::Cmd::Focus, crates/agent-msg/src/status_request.rs::focus_list |
| FND-003 | high | SWM-109 server capacity set/clear and lane grouping display have no owning requirement in `spec/` | crates/agent-msg/src/main.rs::Cmd::ServerCapacity, crates/agent-msg/src/status_request.rs::server_list |

## Coverage

- Reconciliation: `quoin matrix --repo . --json` (quoin 0.28.3); 55 requirements, 358 criteria, 10 statically tagged, 345 untagged, 3 method-without-symbol; all 358 show no run evidence.
- Plan completion: not assessed.
- Untraced changed behavior: 3 feature groups (status request/replies, focus, server capacity); no placeholder source or test implementations found in changed files.
- Semantic review: skipped (optional and not requested).
- Redis integration evidence: unavailable because `IX_RELAY_TEST_URL` is unset. The affected tests compile and are wired to return early when the URL is absent; their runtime behavior remains unverified here.

## Current-head follow-up (`1b21e665673f0213469713a485b01634b597b9ef`)

FR-056-AC-4 now owns the unknown spare-coder-slot representation. Its tagged CLI test verifies that omission produces JSON `null`, while an explicit value remains numeric; the computed matrix marks all four FR-056 criteria tagged. The prior missing-spec findings remain fixed by FR-056 through FR-058. No new specification gap was found.


## Dispositions

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | FR-056 now specifies status request, reply, retention, and collection behavior. Its three acceptance criteria are tagged by the new CLI integration test. |
| FND-002 | fixed | FR-057 now specifies focus set/clear, display, and stale marking. Its three acceptance criteria are tagged by the new CLI integration test. |
| FND-003 | fixed | FR-058 now specifies operator-entered capacity and grouped lane display. Its two acceptance criteria are tagged by the new CLI integration test. |

After excerpts, verified in commit d34cf353:

> **FND-001 after_excerpt:** Given a partly answered request, `status-replies` emits JSON with the request id, requested lanes, replies, and nonresponders.
> **FND-002 after_excerpt:** Given a focus older than the configured stale age, `who --focus --stale-after` marks it `STALE` and still prints the focus.
> **FND-003 after_excerpt:** Given registered lanes on multiple servers, `who --servers` groups their names by server and displays the explicit capacity or unset state.

## Current-head reconciliation

The follow-up proposal to serialize an omitted spare-coder-slot count as JSON
`null` is not adopted: merged FR-056 requires the option and defines the field
as an integer. The implementation and tests retain that contract.
