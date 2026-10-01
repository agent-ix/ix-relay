---
id: SR-058
title: "Herdr payload diagnostics implementation gap analysis"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay@ba4055d343d35b690a4b3d009816c73553aeef1c; crates/herdr-cli/Cargo.toml, crates/herdr-cli/src/lib.rs, crates/relay-ops/src/herdr_check.rs, spec/functional/FR-048-session-work-monitor.md; diff against bcd4f03d7039d48a4a4adde9bebf6e978c2ab337"
review_set: subset
---
## Summary

FR-048-AC-8 states the newly exposed payload failure and the unchanged error classifications. Existing FR-036-AC-2 governs the version/shape diagnostic caller that now reuses the shared implementation. The raw field reader has tests for presence, absence and explicit null; duplicate fields remain subject to the typed payload decoder. There is no permissive reader, guessed identity, status coercion or accelerated retry.

## Verdict

**PASS** — no findings in the reviewed diff.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | - |

## Evidence

Behavioral tests run the real decoder and existing shape-check tests through process-runner seams. They measure actual error text and typed variants, preserving the distinction between parse failure, provider-reported error and nonzero process exit.

## Evidence limitations

The dashboard's transient parse failure recovered on its scheduled minute refresh. Its original malformed response was not captured; this change does not claim to identify or repair that response's producer. It corrects the generic diagnostic that masked the payload error, so a recurrence can be investigated. IR work reports were absent during the check and are independent of Herdr activity. No applicable AssuranceProfile was found. Installed quire has no matrix command, so no strict computed trace matrix is claimed. Spec validation retains existing advisory EARS/module warnings.

## Live verification

After IR published its explicit reports, the restarted dashboard using the new decoder read the same native session and process binding successfully. Its Agents view showed seven assignments (five active, two finished stages) independently of tracker workflow, and the loaded tracker showed 198 of 340 scoped leaves completed across three projects and four associated initiatives. The Scope view showed actual scoped done, remaining, ready and blocked counts. Original absence of work reports is no longer a live blocker. The transient malformed Herdr response itself was not captured.
