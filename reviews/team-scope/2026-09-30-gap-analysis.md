---
id: SR-060
title: "Dashboard team scope gap-analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@bb1639ee43754ede3a8ee645939ecc478a80f8ee; Cargo.toml, Cargo.lock, crates/agent-msg/src/work.rs, crates/relay-core/src/work.rs, crates/relay-core/tests/work.rs, crates/relay-ops/src/work.rs, crates/relay-ops/tests/work.rs, crates/relay-tui/README.md, crates/relay-tui/src/monitor.rs, monitor_main.rs, monitor_tracker.rs, monitor_ui.rs, spec/functional/FR-048-session-work-monitor.md; diff against 8538ddb4115f06de082a52d4c6547b8260032563"
review_set: subset
---
## Summary

FR-048-AC-9 is covered by actual terminal/model tests for mixed-team projects, default team scope including unprojected work, explicit project narrowing, excluded foreign ticket roots, manual team override, unknown team refusal, and team identity while Linear loads. The public CLI parser test refuses absent/conflicting scope choices; the real Redis test clears narrower selectors while preserving agent assignments. Existing initiative progress tests now explicitly select initiatives, and the Scope view still shows per-project progress. FR-048-AC-10 uses the external ProcessRunner seam to test omitted/null argument details with verified identity, refusal without identity, absent process evidence and malformed argument types. Existing measured conflict/replacement tests pass. The compact header contract now matches Scope rows providing per-project counts. The upstream team and project membership algorithms have their own public API acceptance tests and two full gates per PR.

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
