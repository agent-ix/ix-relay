---
id: SR-059
title: "Dashboard team scope code-review"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@bb1639ee43754ede3a8ee645939ecc478a80f8ee; Cargo.toml, Cargo.lock, crates/agent-msg/src/work.rs, crates/relay-core/src/work.rs, crates/relay-core/tests/work.rs, crates/relay-ops/src/work.rs, crates/relay-ops/tests/work.rs, crates/relay-tui/README.md, crates/relay-tui/src/monitor.rs, monitor_main.rs, monitor_tracker.rs, monitor_ui.rs, spec/functional/FR-048-session-work-monitor.md; diff against 8538ddb4115f06de082a52d4c6547b8260032563"
review_set: subset
---
## Summary

Reviewed the complete PR diff against repository conventions and the Rust checklist, including unchanged CI lane definitions. Membership and leaf progress come from the pinned authoritative ix-board API rather than a consumer copy. Team ownership intersects explicit focus; global prerequisite context is retained. Absent reports select team scope without inventing agent activity. Unknown team keys refuse selection. Project focus does not surface every associated initiative as extra focus. All reported assignments remain visible. The compact header retains the known team during source loading and hides zero-count optional selectors. The --all-team command is explicit, conflicts with narrower selectors and changes only existing scope fields. Limits, binding isolation and concurrent agent updates remain enforced. Herdr protocol 22 declares argv optional/nullable and foreground_processes optional; the reader models those current contract states. Missing argument evidence cannot manufacture a native identity, and missing processes cannot establish a binding. Required name/PID and conflicting native identity checks remain intact. Malformed argument types remain BadOutput. The test-only decorator has a localized, explained expect_used expectation for fixed fixture assertions. No compatibility reader, vendored file or CI gate change exists.

## Verdict

**PASS** — no findings in the reviewed diff.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Evidence

21 dashboard tests, two native binding tests, the CLI selection test and scoped Clippy pass. The first full pre-PR gate passed with 1222 tests and 22 ignored. The additional current-contract process test and final source are covered by the second full gate before merge; results are recorded in the PR. Full spec validation has no structural failures, retaining existing advisory module/EARS warnings.

## Evidence limits

No applicable AssuranceProfile exists. Installed quire has no matrix command; no strict computed trace matrix verdict is claimed. Live preview verifies two reported projects with IR-only 195/333 leaf progress, rather than the previous associated initiative and descendant expansion. All active agent tickets in the old report are outside those project selectors; IR has been asked to report its actual lane scope. Report accuracy still depends on the lead. The API contract was read through herdr api schema --json (protocol 22); no provider schema was copied into this repository. One earlier full run hit an unchanged socket fixture timeout; focused retry and the subsequent complete run passed with eight test threads, without disabling any lane or assertion.
