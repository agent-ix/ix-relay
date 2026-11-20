---
id: SR-089
title: "SWM-136 binding code review"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; SWM-136; Cargo.lock, crates/relay-ops/Cargo.toml, crates/relay-ops/src/work.rs, crates/relay-ops/tests/work.rs"
review_set: subset
---

## Summary

Ticket SWM-136; PR ix-relay#38. Reviewed frozen df78cc7bd72a53029a60be641a6341f7bf0cd224 against origin/main (00524e4). All four changed paths inspected. Code-review includes the rust-review lane and rust-style references. No applicable AssuranceProfile or spec/plan changes exist. Exact model identifier unavailable in harness.

## Verdict

PASS for the source change: no defects identified. Live ticket AC-5 is NOT assessed by reviewer; root owns actual authenticated startup and aggregate gate. This is not live acceptance or a whole-repository assurance PASS.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Coverage

Exact basename matching checks optional argv0 OR name once per process, so both matching fields still count as one candidate. Arbitrary argv mentions never identify the executable. Missing/null argv0 and argv deserialize as absence; malformed types fail shared decoding. Session-kind, native identity, nonzero PID, ambiguous process and replacement fences remain intact. Names and argv0 accept executable paths; helper suffixes and parent-directory mentions do not match.

Searched relay-ops for process_identity, argv0 and file_name before assessing duplication; no second implementation introduced. Herdr-owned payloads use the documented tolerant selected-field adapter. No public API, narrowing cast, unsafe code, concurrency, CI lane, coverage threshold, source pin or release/version changed. ix-trace-rs is already locked and source-allowlisted; only relay-ops dev wiring is added.

The fake ProcessRunner replaces the external command seam; the production resolver and decoder run. Positive assertions inspect actual returned SID/PID/kind/address. Negative cases assert typed refusal; removing the executable fallback or identity/type guards would fail assertions. Existing tests exercise Codex resume and PID/SID replacement. Coder red/green is retained in implementation/red.log; reviewer did not mutate frozen source to repeat red.

Reviewer actual exits: make fmt-check 0 (existing nightly-only rustfmt-option warnings visible); focused relay-ops/relay-tui all-target Clippy --locked -D warnings 0; relay-ops work tests 0 (5 passed); dashboard tests 0 (41 passed); make deny 0 (licenses ok). Same retained worktree target used. No make ci, live Herdr, credentials, global environment writes, source edits or second worktree.

## Examined scope

```json
[
  {
    "id": "FR-048-AC-1",
    "path": "spec/functional/FR-048-session-work-monitor.md",
    "role": "examined",
    "excerpt": "Resolve exactly one live agent by address, verify its native session and process, refuse missing/conflicting identity, and require a restart to bind a replacement. Reports are isolated by machine, native session, and process incarnation."
  },
  {
    "id": "FR-048-AC-10",
    "path": "spec/functional/FR-048-session-work-monitor.md",
    "role": "examined",
    "excerpt": "Binding accepts omitted or null argv as specified by Herdr protocol 22 while retaining verified native session and nonzero PID requirements. Omitted foreground-process lists provide no binding evidence; malformed argument types are refused."
  },
  {
    "id": "SWM-136-AC-1",
    "path": "Linear:SWM-136",
    "role": "examined",
    "excerpt": "Recognize the exact agent executable basename from Herdr's optional argv0 as well as process name, retaining native-session and nonzero-PID checks."
  },
  {
    "id": "SWM-136-AC-2",
    "path": "Linear:SWM-136",
    "role": "examined",
    "excerpt": "Ignore unrelated helper processes and reject zero/multiple matching processes; arbitrary mentions in arguments do not establish process identity."
  },
  {
    "id": "SWM-136-AC-3",
    "path": "Linear:SWM-136",
    "role": "examined",
    "excerpt": "Preserve behavior when argv0 or argv is absent/null and refuse malformed types."
  },
  {
    "id": "SWM-136-AC-4",
    "path": "Linear:SWM-136",
    "role": "examined",
    "excerpt": "Tests cover versioned Claude names, executable paths, helpers and ambiguous candidates. Run focused relay-ops and dashboard tests plus Clippy."
  },
  {
    "id": "SWM-136-AC-5",
    "path": "Linear:SWM-136",
    "role": "context_only",
    "excerpt": "Live dashboard opens against the verified Claude session; do not restart or modify the lead."
  }
]
```

## Bindings

```json
[
  {
    "test_id": "claude_binding_matches_exact_executable_basename_with_optional_arguments",
    "ac_id": "FR-048-AC-1",
    "trace": "correct"
  },
  {
    "test_id": "claude_binding_matches_exact_executable_basename_with_optional_arguments",
    "ac_id": "FR-048-AC-10",
    "trace": "correct"
  },
  {
    "test_id": "claude_binding_refuses_helpers_ambiguous_candidates_and_zero_pid",
    "ac_id": "FR-048-AC-1",
    "trace": "correct"
  },
  {
    "test_id": "claude_binding_refuses_helpers_ambiguous_candidates_and_zero_pid",
    "ac_id": "FR-048-AC-10",
    "trace": "correct"
  },
  {
    "test_id": "claude_binding_refuses_malformed_optional_argument_types",
    "ac_id": "FR-048-AC-10",
    "trace": "correct"
  }
]
```

