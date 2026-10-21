---
id: SR-069
title: "Code review — ix-relay SWM-107–109"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/agent-msg/src/main.rs, crates/agent-msg/src/status_request.rs, crates/relay-core/src/status.rs, crates/relay-core/src/lane_registry.rs, crates/relay-core/src/store.rs, crates/relay-core/src/store/status.rs, crates/relay-core/src/store/keys.rs, crates/relay-core/src/lib.rs"
review_set: subset
---

## Summary

Reviewed PR #26 at `99ec6afe7bfa6ae578bf73f56ac6611ed8f3dc80` against the merged FR-056 through FR-058 contract. The status reply requires a numeric `spare_coder_slots` value from 0 through 65535; `--spare-coder-slots` is required, and JSON output retains the number. The focus and explicit server-capacity behavior also match their requirements.

## Verdict

**PASS** — no open findings at the reviewed head.

## Scope

| Unit | Role | Excerpt |
| --- | --- | --- |
| FR-056-AC-1 | examined | Request JSON has a request ID, recipient snapshot, and one delivery outcome per recipient. |
| FR-056-AC-2 | examined | Notices identify the request and provide the reply command. |
| FR-056-AC-3 | examined | Replies carry the required fixed-form fields, including numeric spare coder slots. |
| FR-056-AC-4 | examined | The first valid reply from each requested lane is retained. |
| FR-056-AC-5 | examined | Partial and complete collections return replies and nonresponders. |
| FR-057-AC-1–7 | examined | Focus is explicit, scoped to the caller, and marked stale at the configured boundary. |
| FR-058-AC-1–6 | examined | Capacity is operator-entered and grouped by registered server without inference. |

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | A new test failed the repository's Clippy `-D warnings` gate. | crates/relay-core/src/status.rs:283 |
| FND-002 | medium | Initial tests did not invoke the new CLI commands or verify their output contracts. | crates/agent-msg/src/status_request.rs:42-200 |
| FND-003 | medium | Initial CLI coverage omitted rejection and partial-collection cases. | crates/agent-msg/tests/program_review.rs:45-106 |

## Dispositions

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | `std::slice::from_ref` removes the cloned one-item slice that triggered Clippy. |
| FND-002 | fixed | `program_review.rs` invokes the status, focus, and capacity commands and checks their output contracts. |
| FND-003 | fixed | The status CLI test covers `all`, explicit targets, invalid/duplicate/self targets, unrequested replies, partial collection, and first-write-wins behavior. |

## Exact-head review (`99ec6afe7bfa6ae578bf73f56ac6611ed8f3dc80`)

- Verified the implementation and tests against the merged `spec/functional/FR-056-repeatable-program-review-status.md`, `FR-057-lane-focus-is-explicit-and-visible.md`, and `FR-058-explicit-server-capacity.md`.
- `StatusReply.spare_coder_slots` is a required `u16`; the CLI argument is required and values above 65535 are refused. The reply serializer emits the field as a number, including zero.
- `submitted_at_ms` is required during wire deserialization, while the relay store assigns its received value; the commit removes the serde default and includes a missing-timestamp refusal test.
- The test suite covers exact reply keys, numeric values, missing required CLI arguments, unsafe and oversized items, blocker IDs, duplicate submissions, unrequested responders, and partial collection. Focus tests cover unset state, set/clear isolation, and stale age boundaries. Capacity tests cover explicit set/clear and grouped output.
- The capacity validation no longer imposes undocumented maximum rule count or length limits; empty and unsafe rules remain refused.
- The earlier provisional null-contract review at `1b21e665673f0213469713a485b01634b597b9ef` is superseded by merged FR-056 and the required-numeric implementation at this head. No change to the merged contract is proposed here.
- No new findings. The 99ec6af changes align missing-timestamp handling, criterion trace tags, and owner-rule bounds with the stated requirements. This review inspected implementation and test coverage; it did not independently run the test suite.
