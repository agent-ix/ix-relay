---
id: SR-068
title: "Code review — ix-cli-kit and ix-board pin migration"
type: SpecReview
analysis: code-review
scope: "Cargo.toml; Cargo.lock; crates/relay-tui/src/monitor_ui.rs; .github/workflows/"
review_set: subset
---

## Summary

Reviewed the PLAT-1140 dependency and test-fixture diff against the Rust checklist and unchanged CI workflows. The manifest follows ix-cli-kit `main`; Cargo.lock resolves it once to `be26cf02836e25f8ec7efa6de51e974d331a5816`. The updated ix-board revision required explicit additions to dashboard test fixtures. Focused dashboard tests and one complete `make ci` passed. Newly selected TLS packages were advanced to advisory-fixed compatible versions, and `cargo audit --no-fetch` reports zero vulnerabilities.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | low | No findings (placeholder) | Cargo.toml; Cargo.lock; crates/relay-tui/src/monitor_ui.rs |

## Verdict

No findings in the reviewed diff. The model fields in test fixtures reflect ix-board's current structured records, and the locked graph has one authoritative ix-cli-kit source.
