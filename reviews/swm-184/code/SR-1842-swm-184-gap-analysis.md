---
id: SR-1842
title: "Gap analysis - FR-048, FR-059, FR-061 and FR-064 criteria touched by the ix-board c816c187 consumer fix, spec criteria to tests to code (SWM-184)"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-048-session-work-monitor.md; spec/functional/FR-059-department-monitor-adapter.md; spec/functional/FR-061-named-focus-definitions.md; spec/functional/FR-064-named-focus-dashboard-reader.md; crates/relay-tui/src/monitor.rs; crates/relay-tui/src/monitor_ops.rs; crates/relay-tui/src/monitor_ui.rs; crates/agent-msg/src/focuses/resolve.rs; crates/agent-msg/src/focuses/tests.rs; crates/agent-msg/src/work.rs"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-048", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-059", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-061", type: references }
  - { target: "ix://agent-ix/ix-relay/FR-064", type: references }
---

## Summary

Ticket: SWM-184. PR pending. Frozen source HEAD 38b0fc4 against base 7278c1d. Planless, repository-driven gap analysis scoped to the criteria this change touches: FR-048-AC-3 and AC-5 (and the 30-minute and rate-limit paragraph that extends AC-5), FR-059-AC-5, FR-064-AC-10 and FR-061-AC-19. Plan completion was not assessed; this is a repository claim about those criteria and nothing more. FR-065 (view selector), FR-060 and the named-focus activation work are unchanged by this diff and are not examined.

The change adds no new requirement, and every behavior it adds has an owning criterion: the retained-snapshot notice is FR-059-AC-5 and FR-064-AC-10, rate-limit protection is FR-048-AC-5, and the eight typed-error placements are FR-061-AC-19. No source or test stub backs a criterion. The audit found 3 gaps, all low: how FR-048-AC-5 is bound by tests, local-budget exhaustion protected only when no snapshot exists, and a work-report path with no criterion or test for a retained snapshot. None is a missing feature.

## Verdict

**CONDITIONAL** - no `high` finding and no untagged criterion that I could establish; 3 low findings. A planless CONDITIONAL asserts repository assurance for the listed criteria only (traceability, reverse gap, stubs) and nothing about plan completion. The matrix was not run by me: if the parent's `quire matrix --strict` reports FR-048-AC-5 as untagged, the verdict rule makes this FAIL, and FND-001 is the cause. The defects in the code itself (SR-1841 FND-001 and FND-002) are not repeated here.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | low | FR-048-AC-5: none of its nine test citations in the crates is a `#[trace]` attribute, so by this repository's own matrix reading it may be untagged, and the new local-budget test adds a ninth docstring-only citation | FR-048-AC-5, crates/relay-tui/src/monitor_ops.rs:560 |
| FND-002 | low | FR-048-AC-5: local-budget exhaustion is protected only when no snapshot exists; with a retained snapshot the SDK reports it as `stale` without a cooldown, so refreshes do not coalesce, while the new test and its docstring claim the protection generally | FR-048-AC-5, crates/relay-tui/src/monitor_ops.rs:556 |
| FND-003 | low | FR-048-AC-3: resolving scope and verifying tickets for `agent-msg work` has no criterion or test for a retained snapshot after a failed refresh, and `source_snapshot` has no seam to test one | FR-048-AC-3, crates/agent-msg/src/work.rs:86 |

## Coverage

- Reconciliation: no `quire matrix` or `quoin matrix` output was supplied to me and I did not run one. The reading below is a provisional grep index of trace tags in the changed crates, declared as such; it cannot establish an engine-backed PASS. Tag index at the frozen source: FR-061-AC-19 bound by `#[trace]` at agent-msg focuses/tests.rs:681, 720, 764, 1716 and 1821; FR-059-AC-5 bound at relay-tui monitor_department.rs:1105 and monitor_ops.rs:354; FR-064-AC-10 bound at monitor_ops.rs:354 only; FR-048-AC-5 has no `#[trace]` attribute anywhere under crates/ and nine `/// Trace:` citations (monitor_ops.rs:524, 560, 585, 622; monitor_runtime.rs:104; monitor_ui.rs:1073, 1253, 1275, 1624); FR-048-AC-3 is cited only in `/// Trace:` docstrings at monitor_ui.rs:1356, 1624 and 1649, none in agent-msg or relay-ops. SR-122 FND-010 in this repository records quire reporting a docstring citation as `a comment never binds`; the rust-style testing reference accepts both forms. The parent's matrix decides.
- Plan completion: not assessed
- Criteria examined: FR-048-AC-3, FR-048-AC-5, FR-059-AC-5, FR-064-AC-10, FR-061-AC-19. Other criteria of these requirements and the legacy rows of the full-repository matrix are not assessed and are not clean assurance.
- Evidence: no run evidence read. The parent reports fmt exit 0 and focused clippy exit 0 at this source and focused tests still running; I ran none of these and claim no fresh green.
- Untraced behaviors / stubs: inventoried 7 behaviors of the change (retained notice with a cooldown, retained notice without one, original `fetched_at` kept, protected refresh for a retained cooldown, `LinearLocalBudgetExhausted` Err arm and its text, the eight placements in `reason_of`, the carried provenance); untraced 1 (the local-budget text and its behavior on the retained path, FND-002); source stubs 0; test stubs 0. No `TODO`, `FIXME`, `todo!`, `unimplemented!` or new `#[ignore]` in the changed files.
- Semantic review: not run as the separate opt-in pass. The per-criterion judgement below is the reviewer's reading of the tests, not that step.
- Upstream: the accepted SDK at c816c18 was read only where the consumer touches it; an exhaustive upstream audit was not assessed.

### Per-criterion assessment

| AC | Evidence assessed | Strength |
|---|---|---|
| FR-048-AC-3 | scope resolution through ix-board in `agent-msg work` and relay-ops `resolve_work_scope`; no test of a retained snapshot | partial (FND-003; defect SR-1841 FND-002) |
| FR-048-AC-5 | retained snapshot keeps its label and original time; rate-limit protection and coalescing for the SDK cooldown; local budget on the cold Err arm | partial (FND-001, FND-002) |
| FR-059-AC-5 | `a_snapshot_kept_after_a_failed_refresh_shows_the_failure_and_its_original_time` drives a real `ReadService` over a temp cache through good, rate-limited, failed and recovered reads; UI test shows the row and hides the raw cause | strong for failures with a known age; the unknown-provenance branch is SR-1841 FND-001 |
| FR-064-AC-10 | same test: last good result and its `fetched_at` retained after a failed refresh, notice shown at once on a cold app | strong, same note |
| FR-061-AC-19 | table test places every SDK variant, with the eight new ones; bounded-read tests at 682, 721, 765 and 851 keep the 300 s age, no-stale and deadline behavior against the new pin | strong |

### Bindings checked

The three bindings listed in the comment payload were read in the test bodies; none is tagged to the wrong criterion.

## Finding details

### FND-001 (low) FR-048-AC-5 is bound only by docstring citations

FR-048-AC-5 requires labeled last snapshots on failure and, in the paragraph that extends it, coalesced manual refreshes during rate limiting. Nine tests in the crates cite it with a `/// Trace: FR-048-AC-5` line and none with `#[trace("FR-048-AC-5")]`; the new `local_budget_exhaustion_is_protected_like_a_rate_limit` (monitor_ops.rs:556-584) is the ninth, and the new `a_retained_snapshot_shows_the_failed_refresh_while_under_thirty_minutes_old` (monitor_ui.rs:1323-1329) cites FR-059-AC-5 and FR-064-AC-10 the same way. `a_snapshot_kept_after_a_failed_refresh_shows_the_failure_and_its_original_time` carries both a docstring line and the attribute (monitor_ops.rs:353-354), which is the pattern SR-122 FND-010 reported as adding `trace-id-outside-marker` warnings. If quire does not bind a Rust docstring line, FR-048-AC-5 is untagged in the computed matrix; the baseline already had this, so the gap pre-exists and the PR extends it. Confidence is low because I did not run the matrix. Request: bind the new tests with `#[trace(...)]` only, drop the docstring lines on them, and add the attribute to the existing rate-limit tests or confirm in the parent's `quire matrix --strict` that FR-048-AC-5 is tagged.

### FND-002 (low) Local-budget protection exists only on the cold-start path

The paragraph extending FR-048-AC-5 says manual refreshes during rate limiting coalesce until the next attempt. The consumer now treats `LinearLocalBudgetExhausted` as protected on the Err arm (monitor.rs:303-311, monitor_ops.rs:214-223) and `local_budget_exhaustion_is_protected_like_a_rate_limit` asserts that. In the SDK, once any complete snapshot is stored, a refresh that fails with that error returns the snapshot as `Ok` and sets `rate_limit_until` only for `LinearRateLimited` (lib.rs:981-1008). So with a snapshot, which is the normal case after the first read, a local-budget refusal arrives as `stale` with no cooldown: `protected` is false, a manual `r` is not coalesced, no countdown shows, and the notice says `Linear refresh failed; ... Inspect source availability with ix-board`. Each extra refresh is refused locally before any request is sent, so no quota is spent; the harm is a wrong remedy and a spec paragraph the path does not meet. The typed signal that would distinguish it is not in `CacheFreshness`, and parsing `failure_reason` is ruled out, so the honest outcomes are neutral wording on the retained path or a request to ix-board for a typed cause. The test docstring (`It is protected like a server rate limit: manual refreshes coalesce, the countdown shows`) claims more than the retained path delivers, and no requirement or document names the local budget or the text `Linear request budget exhausted`. Request: decide the retained-path behavior and record it in the AC or docs; limit the test and its docstring to the cold-start Err path it exercises.

### FND-003 (low) The work-report resolution path has no criterion or test for a retained snapshot

FR-048-AC-3 says scope is resolved through ix-board and that workflow state comes from the same Linear snapshot. `agent-msg work` resolves scope and verifies tickets through `source_snapshot()` (work.rs:86-89), which hard-wires `ReadService::open_default(LinearGraphQlSource::from_env()?)` and has no injected source, so no test can present it a retained snapshot, and no criterion says whether a command that writes a relay record may rest on one. This is the missing oracle for the defect in SR-1841 FND-002; the two are different units (the code path there, the criterion and seam here). Request: when FND-002 of SR-1841 is decided, state the chosen behavior in FR-048-AC-3 or a new criterion and add a test through a seam such as a `WorkspaceSource` passed in by the caller.
