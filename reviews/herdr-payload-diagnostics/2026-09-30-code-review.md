---
id: SR-057
title: "Herdr payload diagnostics code and Rust review"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@ba4055d343d35b690a4b3d009816c73553aeef1c; crates/herdr-cli/Cargo.toml, crates/herdr-cli/src/lib.rs, crates/relay-ops/src/herdr_check.rs, spec/functional/FR-048-session-work-monitor.md; diff against bcd4f03d7039d48a4a4adde9bebf6e978c2ab337"
review_set: subset
---
## Summary

Reviewed the source diff against the repository conventions and Rust checklist. The shared decoder selects the external envelope separately from the typed command payload. RawValue retains duplicate keys and payload nullability without copying a provider schema or emitting an untyped wire format. The custom field reader records presence, distinguishing an explicitly present null from a missing result. The typed payload remains subject to serde validation. Reported errors take the same precedence as before, nonzero exits retain UnexpectedExit, and call still supplies the measured exit status. The version/shape check now uses this shared typed path. No runtime threads, polling schedule, binding policy, compatibility mechanism or CI gates changed.

## Verdict

**PASS** — no findings in the reviewed diff.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Evidence

121 focused tests passed across herdr-cli and relay-ops, and scoped all-target Clippy passed. Tests assert missing fields, invalid field types, duplicate rejection, explicit null versus missing result, extra envelope metadata, valid typed payloads, reported errors and nonzero-exit classification. Existing FR-036 shape checks still name missing state_change_seq. Full gates are recorded in the PR.

## Evidence limitations

The dashboard's transient parse failure recovered on its scheduled minute refresh. Its original malformed response was not captured; this change does not claim to identify or repair that response's producer. It corrects the generic diagnostic that masked the payload error, so a recurrence can be investigated. IR work reports were absent during the check and are independent of Herdr activity. No applicable AssuranceProfile was found. Installed quire has no matrix command, so no strict computed trace matrix is claimed. Spec validation retains existing advisory EARS/module warnings.
