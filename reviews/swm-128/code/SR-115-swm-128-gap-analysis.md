---
id: SR-115
title: "Gap analysis - FR-061 named focus definitions, spec criteria to tests to code (SWM-128)"
type: SpecReview
analysis: gap-analysis
scope: "agent-ix/ix-relay; spec/functional/FR-061-named-focus-definitions.md; crates/relay-core/src/named_focus/**; crates/relay-core/src/store/named_focus.rs; crates/relay-core/src/store/named_focus_scripts.rs; crates/relay-core/tests/named_focus.rs; crates/agent-msg/src/focuses.rs; crates/agent-msg/src/focuses/**; crates/agent-msg/tests/focuses_cli.rs"
review_set: subset
relationships:
  - { target: "ix://agent-ix/ix-relay/FR-061", type: references }
---

## Summary

Ticket: SWM-128. PR ix-relay#42 at immutable source 663425a106821d4b384666e7ee012b6410bf11f2. Planless, repository-driven gap analysis of the assigned scope: the 22 acceptance criteria of FR-061 against the tagged tests and the source. Plan completion was not assessed; this is a repository claim about FR-061 only. FR-062 (activation), FR-063 (evaluation) and FR-064 (dashboard) are later planned tickets: their missing implementation is not a finding here, and no finding below asks for it.

All 22 criteria carry at least one `#[trace]` tag on a running test, and no source or test stub backs any of them. The audit found 9 gaps: 3 medium and 6 low. They are untested branches of stated criteria (the simultaneous-claim race of AC-8, the create-path duplicate name of AC-4, the update-time reclamation trigger) and a few contract-versus-behavior seams, not missing features. Nothing in the source is unspecified in a way that creates user-visible behavior without an owner except the small set in FND-009.

## Verdict

**CONDITIONAL** - no untagged criterion and no `high` finding; 3 medium and 6 low findings. A planless CONDITIONAL asserts repository assurance for FR-061 only (traceability, reverse gap, stubs) and nothing about plan completion.

## Findings

| ID | Severity | Summary | Refs |
|---|---|---|---|
| FND-001 | medium | FR-061-AC-8: no test makes two claims race from the same observed owner and epoch, and none asserts that the loser's owner_epoch_conflict carries the new owner and epoch | FR-061-AC-8, crates/relay-core/tests/named_focus.rs:666 |
| FND-002 | medium | FR-061-AC-4: a duplicate name refused with name_taken is tested only on update; create-path duplicates (including case-insensitive and concurrent) are never exercised | FR-061-AC-4, crates/relay-core/tests/named_focus.rs:416 |
| FND-003 | medium | FR-061-AC-16: reclamation trigger (b) "at each update, before older revisions are trimmed" is implemented and has no criterion and no test | FR-061-AC-16, crates/relay-core/src/store/named_focus.rs:980 |
| FND-004 | low | FR-061-AC-7: three stated branches are not executed (a claim that succeeds on a replaced owner, a resumed process at a new address staying live, an owner on another machine with unverified placement) | FR-061-AC-7, crates/relay-core/tests/named_focus.rs:562 |
| FND-005 | low | FR-061-AC-17: the "at most 8 examined" bound and an unreadable pin on a removal candidate have no test | FR-061-AC-17, crates/relay-core/tests/named_focus.rs:1128 |
| FND-006 | low | FR-061-AC-19: the `stale` reason of source_unavailable has no producer, so the older-than-five-minutes clause is met only by refresh | FR-061-AC-19, crates/relay-core/src/named_focus/error.rs:96 |
| FND-007 | low | FR-061-AC-18: the JSON listing of an unavailable definition carries no `unavailable_definition` code, and an unknown hash field of the record has no test | FR-061-AC-18, crates/agent-msg/src/focuses/output.rs:217 |
| FND-008 | low | FR-061-AC-22: the tests tagged to the criterion check only that clap still parses the old commands, not their behavior and bytes | FR-061-AC-22, crates/agent-msg/src/focuses/tests.rs:1487 |
| FND-009 | low | Reverse gap: five implemented behaviors have no owning requirement (a 256 KiB definition-file bound reported as a limit, transfer to the current owner refused, claim of an archived definition, one name in several teams refused, a 128-field record cap) | crates/agent-msg/src/focuses/file.rs:35 |

## Coverage

- Reconciliation: quire matrix run by the root (prefreeze-matrix.json, reported by the root as FR-061 22 of 22 criteria tagged). I did not run quire or quoin. The saved matrix is a single JSON line and my read tool omits long matching lines, so I could extract only short fields from it. I cross-checked tag presence by reading every `#[trace(...)]` attribute in the three FR-061 test files. All 22 ids appear: AC-1 and AC-5 to AC-18 in tests/named_focus.rs or focuses/tests.rs; AC-2, AC-19, AC-20 and AC-22 in focuses/tests.rs; AC-21 in focuses/tests.rs and focuses_cli.rs. This is a grep cross-check of tags, not an engine figure, and a tag proves traceability, not that the test checks the criterion.
- Plan completion: not assessed
- Criteria (FR-061, root's matrix, not recomputed by me): tagged 22 · untagged 0 · tagged-by-ignored-test 0 · method-without-symbol 0. Other requirements and the legacy or future rows of the full repository matrix are not assessed and are not clean assurance. I could not parse that data with the tools available to me and claim no classification of those rows.
- Evidence: no run evidence (quoin matrix not run). The root's logs report a green focused run: 27 store tests (isolated Redis, not skipped), 21 focuses tests, 5 process tests plus the existing work_json test, 5 key tests. The initial core unit run at f4f6007 failed one key-boundary test; the latest key subsuite is green, and no fresh whole-library green is claimed.
- Untraced behaviors / stubs: inventoried behaviors 20 command-and-option items plus the 18 public `focus_*` store methods; untraced 5 (FND-009); source stubs 0; test stubs 0. No TODO, FIXME, `todo!`, `unimplemented!` or `#[ignore]` in the changed files. Two process tests print a skip note and return when `IX_RELAY_TEST_URL` is unset; that is the repository convention (`make test-redis`) and the root ran them against an isolated Redis.
- Semantic review: not run as the separate opt-in pass. The per-criterion judgement below is the reviewer's reading of the tests, not that step.

### Per-criterion assessment (tag present on all 22)

| AC | Evidence assessed | Strength |
|---|---|---|
| 1 | create resolves names to canonical IDs, `show --json` round trip as an update file, text forms | strong (labels are canonical IDs, see SR-114 FND-002) |
| 2 | four refusal cases through the real SDK path, no key written | strong |
| 3 | threaded race, stale expectation, identical content, overflow planted at u64::MAX | strong; the script-level revision branch is not forced (SR-114 FND-003) |
| 4 | rename frees name, update-path duplicate, other team, name moved | partial: create-path duplicate missing (FND-002) |
| 5 | strict file parser cases, empty inclusion list | strong |
| 6 | other session, same address with another session, resumed process refresh | strong |
| 7 | live, unknown, unverified claims refused; absent claim; forced claim | partial (FND-004) |
| 8 | forced claim, stale epoch, wrong observed owner; transfer-versus-claim count | weak (FND-001) |
| 9 | stale update, archive and transfer plans after a transfer; transfer-versus-claim | partial: owner-ID branch only (SR-114 FND-003) |
| 10 | zero and several targets, stale epoch, unplaced target, old and new owner | strong |
| 11 | update, transfer, pin, archive refused; read and list --archived; name reusable | strong |
| 12 | boundary create with 128 selectors and 64-byte labels, one over each bound, 56 KiB encode edge | strong (the 56 KiB bound is reachable only at encode level) |
| 13 | 129th active and 257th total, nothing evicted | strong |
| 14 | 20 updates keep 8, PTTL -1 on every key | strong (no clock is needed, none is faked) |
| 15 | one pin per incarnation, 33rd refused, replace at limit, compare-and-delete | strong |
| 16 | absent, other-session, live, failed read, unverified, other machine, real skipped record | strong for the pin-put trigger; update trigger untested (FND-003) |
| 17 | live-pin kept, next oldest removed, dead pin reclaimed then removed, nothing removable refused | partial (FND-005) |
| 18 | unknown members kept in meta and owner, undecodable and unsupported listed, writes refused, unparseable pin kept | partial (FND-007) |
| 19 | four-minute cache without a read, missing, partial, failed, refresh of a stale entry | partial: no `stale` reason (FND-006) |
| 20 | default, 1 and 600 second deadlines with the real `ReadService` and a manual clock, cache-hit resolution timing out, no second read | strong |
| 21 | compact one-line outputs, typed errors, untouched keys outside `focus:*`, real binary on unreachable relay | strong |
| 22 | clap parse of focus, work focus, work show, separate family, `--help` | weak (FND-008) |

## Finding details

### FND-001 (medium) FR-061-AC-8: the simultaneous-claim criterion is not exercised

The criterion has four parts: two simultaneous claims leave one winner at epoch 2 and a loser whose `owner_epoch_conflict` carries the new owner and epoch; a claim with a different observed owner ID is refused even when the epoch matches; the claimer's own ID is never compared; the AC-7 fences still apply. Tests tagged AC-8: `a_forced_claim_takes_a_live_owner_with_a_reason_and_records_it` (tests/named_focus.rs:666, one claimant), the CLI `claim_needs_a_gone_owner_and_force_needs_a_reason` (focuses/tests.rs:996, claims in sequence), plus the wrong-observed-owner and stale-epoch checks inside the AC-7 test (tests/named_focus.rs:562). Not tested anywhere: two claim plans from different sessions committed against the same observed owner and epoch, and any assertion on `current_owner_id` or `current_owner_epoch` of an `owner_epoch_conflict` (the CLI test checks `code` only). The transfer-versus-claim test counts `Applied` and never reads the loser's error. Risk: a regression that drops the winner's identity from the conflict, or lets both claims through, is not caught for the headline scenario. Test request for the same coder: (1) in tests/named_focus.rs plan two claims (sess-b and sess-c, observed `terra/sess-a`, epoch 1, evidence from `nobody()`), commit both, assert exactly one `Committed::Applied` and the other `Err(OwnerEpochConflict { owner_id, epoch })` naming the winner at epoch 2, with the store unchanged by the loser; (2) a threaded variant on two `another_connection()` relays with a Barrier, as the update race does; (3) in focuses/tests.rs, after sess-b claims, run `--json claim <id> --expected-owner-epoch 1` as sess-c and assert `code`, `current_owner_id` `terra/sess-b` and `current_owner_epoch` 2.

### FND-002 (medium) FR-061-AC-4: create-path name_taken is untested

AC-4 requires "a duplicate name in the same team is refused with `name_taken`". The only assertions of `NameTaken` are on update (tests/named_focus.rs:425, focuses/tests.rs:930) and on a moved name (tests/named_focus.rs:482). The CREATE script's own `name_taken` reply (named_focus_scripts.rs:131, named_focus.rs:585-592) is never reached by a test; neither is a case-insensitive duplicate on create (`Plan` then `plan`), nor two simultaneous creates of one name. Risk: the first thing a user does to hit this rule is `create` twice; a regression in the create index or in the case folding would pass the suite. Test request: create `Plan`, then create `plan` in the same team, expect `name_taken` with `holder` the first ID and a byte-identical dump; the same name in team OTHER succeeds; two threads create `Same` together, exactly one succeeds; one CLI case printing the JSON error with `name` and `focus_id`.

### FND-003 (medium) FR-061-AC-16: reclamation trigger (b) has no criterion and no test

FR-061 Adoption pins: reclamation runs "(b) at each `update`, before older revisions are trimmed". The code does it (`focus_update` calls `focus_reclaim_pins` before each commit, named_focus.rs:980-1010). AC-16 exercises trigger (a) only (pin put at the adopters limit) and AC-17 trigger (c) (definitions examined for removal). Nothing exercises (b), and no criterion names it. Risk: dropping the call keeps every test green while dead pins on a definition that is updated but never at the limit stay until it is archived and aged, exactly the leak the requirement closes. Test request: plant pin fields for dead sessions with `set_raw_field` on a non-archived definition, run `update` with `nobody()` evidence and expect the pins gone in the same call; with live evidence and with `unreadable()` evidence expect every pin kept. Whether to add a criterion for (b) is a spec decision, not made here.

### FND-004 (low) FR-061-AC-7: three stated branches are not executed

The test has the owner live, unknown (failed read) and unverified refuse a plain claim, an absent owner claim, and `replaced` only as an `owner_state` assertion (tests/named_focus.rs:603-606). Not executed: a claim that succeeds for a replaced owner (the `is_gone` path with `Replaced`); the same native session resumed at a new address staying `Live` and refusing the claim (the test registers the session at the owner's recorded address, so the address-change case is untested); an owner on another machine whose registration has unverified placement (the pin test covers that machine split, the claim test does not). Fix: three small claim cases with the matching `LivenessEvidence`.

### FND-005 (low) FR-061-AC-17: two stated limits are not tested

"At most 8 archived definitions" examined, oldest first: no test puts 8 undeletable (live-pinned) archived definitions ahead of a removable ninth and expects `limit`. An unreadable pin on a candidate (kept, never interpreted) is covered for reclamation (tests/named_focus.rs:1567) but not for removal, where the script treats any `pin:` field as a pin. Fix: add both cases to the removal test.

### FND-006 (low) FR-061-AC-19: the `stale` reason cannot occur

FR-061 and the error table list `stale` as a `source_unavailable` reason, and AC-19 says an older-than-five-minutes snapshot "is refused ... with the matching `reason`". The accepted SDK refreshes an entry older than `max_age` under the same budget and "never returns a stale entry" (ReadService docs), so the relay can only see `missing`, `partial`, `failed` or `timeout`. `SourceReason::Stale` (error.rs:96, 111) has no producer, the docs omit it, and the test named for the case (focuses/tests.rs:691) asserts only the refresh and `failed`. The behavior meets "an older snapshot SHALL NOT validate a selection", so this is a contract-wording gap, not a code defect. Resolution options: record the SDK-refresh reading in FR-061 (a spec ticket), or delete the unreachable variant and say so in the docs. Expected disposition: `deferred` or `accepted-no-change` with that note.

### FND-007 (low) FR-061-AC-18: unavailable definitions in JSON, and one untested tolerance

AC-18: unavailable definitions "are each listed as `unavailable_definition` by ID with their reason". Text output prints `<id> unavailable_definition: <reason>`; the JSON row is `{focus_id, available: false, reason}` (output.rs:217-222) with no `code`, and the test asserts `available` and `reason` only (focuses/tests.rs:1250). A consumer of `--json` cannot tell the stable code. Also untested: an unknown hash field of the record (the criterion says "unknown member of a record"); the tests add unknown members only inside the `meta` and `owner` JSON. Fix: add `"code":"unavailable_definition"` to the row and assert it; add one unknown-field case and assert it survives an update.

### FND-008 (low) FR-061-AC-22: behavior and bytes are asserted by no tagged test

The two tagged tests parse `focus`, `work focus`, `work show` with clap and check `--help` (focuses/tests.rs:1487, focuses_cli.rs:219). The existing byte-level tests for `focus` (program_review.rs:412) and `work show` (work_json.rs:37) are not tagged to AC-22. Production code for the three commands is untouched by the diff (main.rs only adds the `Focuses` variant and routing), so the risk is low. Fix: tag the existing tests to AC-22, or add one process test asserting the exact stdout of `work show` before and after.

### FND-009 (low) Reverse gap: behavior without an owning requirement

Implemented and user-visible, but no FR-061 text owns it: (1) a 256 KiB bound on the definition file, reported as a `limit` error named "definition file bytes" (file.rs:35, 75-82), absent from the limits table; (2) `transfer` to the current owner is refused as `invalid_argument` (named_focus.rs `focus_plan_transfer`); (3) `claim` of an archived definition and `claim` by the current owner are accepted (the FR bans update, activation, adoption and transfer for archived, and is silent on claim); (4) one name in several teams makes a by-name lookup `invalid_argument`; (5) a record over 128 fields is reported `undecodable` (`MAX_RECORD_FIELDS`), which is also what an older relay would say about a newer machine's record with extra fields. None is wrong; each is an unstated rule a second implementer would choose differently. This review authors no requirement. Fix: state them in FR-061 (a spec ticket) or drop them; at minimum put (1) and (5) in the docs.

## Verdict after disposition pass 1

Not run. Dispositions are reserved for this same session after the fix round.
