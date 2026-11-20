---
id: SR-090
title: "SWM-136 binding gap analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; SWM-136 changed binding surface; repository matrix and provisional source inventory"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay"
    type: references
---

## Summary

Repository-driven planless gap-analysis procedures ran on frozen df78cc7bd72a53029a60be641a6341f7bf0cd224. No new binding defect was found; repository assurance remains INCOMPLETE due to existing extraction/trace gaps and incomplete repository-wide reverse mapping. No broad remediation or new ticket is claimed.

## Verdict

INCOMPLETE for whole-repository assurance. The changed binding surface has no identified gap and is owned by existing FR-048-AC-1/10 and SWM-136 AC-1/4. Source review may proceed independently; this artifact does not certify repository assurance or live ticket AC-5.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Coverage

Plan completion: not assessed

Reconciliation: quire matrix (quire 0.52.0, engine 0.62.2) — no run evidence read. quoin 0.36.7; module spec-artifacts-process@v0.29.2. Actual matrix and coverage exits 0. Retained matrix.json and coverage.json are tool outputs, not a hand-authored matrix.

The engine matrix reports tagged 8, untagged 378, method-without-symbol 3; no ignored-only status occurs. These extraction results are not a full coverage claim. Coverage census separately reports 1230 Rust candidate symbols, 712 tagged and 21 bound; legacy comment tags do not bind under the active Rust attribute grammar. NFR section-matches-nothing diagnostics also limit extraction.

The engine binds the two new positive/refusal tests to FR-048-AC-1. Existing FR-048 table is split by blank lines before AC-8/9/10, so AC-10 is not minted and its three new attribute tags appear in untracked_symbols. This is a preexisting extraction limitation; the requirement exists and all three bodies are manually checked against its text. The binding records below mean correct requirement ownership, not engine-backed extraction for AC-10. Legacy bindings and unrelated assurance are left incomplete per dispatch.

Reverse-gap/stub procedure: source-inventory.txt retains public-function and suspicious-pattern inventory across crates. The changed surface has one existing resolver behavior, zero new public behaviors, zero unowned changed behaviors, zero confirmed changed source/test stubs. All three new tests exercise real resolver decoding through an external fake; no bypass, unconditional skip, weak-only assertion, or coverage inflation found. Whole-source per-behavior ownership reconciliation was not completed; do not infer zero repository gaps from this bounded inspection.

Optional semantic-review extension skipped (not requested); ordinary code-review faithfulness checks still ran. No plan supplied. Live dashboard AC-5 NOT assessed by reviewer. Root owns authentic startup evidence and the aggregate gate.

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

