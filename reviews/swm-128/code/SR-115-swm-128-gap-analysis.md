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

*(Round 0 placeholder, kept as written. It is superseded by "Current verdict after disposition pass 1" at the end of this file; disposition pass 1 has now run.)*

## Dispositions

Disposition pass 1 (round 1), same reviewer session as round 0. Source frozen at f813c43e48a18d73d93f3043f6a3e6b7d8a69c00. The original findings, severities, excerpts and history above are unchanged. Cites are f813 lines I read in this pass.

| FND | outcome | reason |
|---|---|---|
| FND-001 | fixed | Three tests added, all tagged AC-8: `of_two_claims_made_at_the_same_epoch_one_wins_epoch_two_and_the_other_is_told_who_won` (tests/named_focus.rs:1900, plans two claims from the same observed owner and epoch, loser gets `OwnerEpochConflict` naming the winner at epoch 2, dump unchanged), the threaded `two_claimants_racing_on_two_connections_yield_one_owner_and_one_conflict` (1958, which re-reads the stored owner as the winner at epoch 2, 2005-2006), and the CLI `the_second_of_two_claims_made_at_the_same_epoch_loses_and_is_told_who_won` (focuses/tests.rs:2020). The script path they exercise is focus_claim.lua:4-8. I read the test names, tags and the end of the threaded test; I did not read every assertion line of the CLI case. |
| FND-002 | fixed | `a_second_create_of_a_name_in_any_letter_case_is_name_taken_naming_its_holder` (tests/named_focus.rs:1841), `of_two_concurrent_creates_of_one_name_exactly_one_definition_exists` (1864, the loser gets `NameTaken` with the winner as holder, 1888) and the CLI `creating_a_second_definition_with_a_name_in_use_is_name_taken_in_any_letter_case` (focuses/tests.rs:1995). The script reply they reach is focus_create.lua:6-7 and the Rust mapping named_focus.rs:585-594. |
| FND-003 | fixed | `an_update_reclaims_pins_of_absent_sessions_before_trimming_and_keeps_live_and_unknown_ones` (tests/named_focus.rs:2374, tagged AC-16). No spec change was made and none is needed to close this finding: the criterion for trigger (b) remains a spec decision outside this PR, as I said in round 0. |
| FND-004 | fixed | `a_replaced_owner_can_be_claimed_but_a_resumed_live_owner_and_a_remote_unverified_one_cannot` (tests/named_focus.rs:2243), tagged AC-7, covers the three branches I listed. |
| FND-005 | fixed | `create_at_the_limit_examines_eight_archived_definitions_and_no_more` (2432) and `a_removal_candidate_with_a_pin_nobody_can_read_is_kept` (2478), both tagged AC-17. |
| FND-006 | accepted-no-change | I approve this independently, on four checks and not on the author's report. (1) The accepted ix-board SDK has no stale-snapshot outcome: the saved 9c source's `Error` has no variant for it, and the only stale wording there is its own rule that a failed forced refresh "leaves the cache empty so later reads cannot use stale data". It refreshes an entry older than `max_age` under the same deadline (the policy the relay passes, resolve.rs:343-346), so a snapshot older than five minutes is never handed to the relay. (2) The relay therefore cannot see an old snapshot, validate against it, or label it; it adds no raw reader, cache reader or timer, which the FR forbids ("The relay adds no Linear reader, token handling or cache"). A search of the relay code finds `Stale` only in the `SourceReason` catalog (error.rs:96, 111). (3) AC-19's clause is met by the stronger path: an old entry is refreshed, and if the refresh fails the user gets `missing`, `partial`, `failed` or `timeout` for the real cause, which is what the "matching reason" can be; `reason_of` places every SDK error under one of those four (resolve.rs:109-171) and the existing test covers the refusal. (4) `SourceReason::Stale` stays in the catalog so the contract's list is complete, and the CLI reference says what is returned (docs/cli-reference.md:698). No ticket and no deferral is needed. Not a blocking contract decision. Optional, at the coder's discretion and not required: one line on the variant's doc comment (error.rs:95), which still says "The snapshot is older than the five-minute bound", noting that no code path produces it today. |
| FND-007 | fixed | The JSON row carries `code: FocusErrorCode::UnavailableDefinition.as_str()` (output.rs:228-233, 265-270) and the test asserts it (`an_unreadable_definition_is_listed_with_its_reason_and_refuses_writes`, per the author; I confirmed the row in source and that tests/named_focus.rs:1740-1755 check the code for the three unavailable cases). The unknown hash field is `an_unknown_field_of_the_record_survives_every_write_and_reclamation` (tests/named_focus.rs:2559, AC-18). |
| FND-008 | fixed | `FR-061-AC-22` was added to the Trace line of the two existing byte-level tests, `tc_057_focus_commands_set_clear_and_render_stale_focus` (tests/program_review.rs:410) and `work_show_prints_the_existing_compact_report_and_one_newline` (tests/work_json.rs:8), which is the repo's idiom for those files. Both return early without `IX_RELAY_TEST_URL`, as the repo's Redis tests do; AC-22 evidence from them exists only in a run with an isolated Redis, which the parent's run has. |
| FND-009 | fixed | Closed at the minimum I stated in round 0 (document (1) and (5)), and I checked that each of the five has a genuine owner, so that no spec wave is needed. (1) 256 KiB file bound: owned by NFR-002 ("no input ... makes a tool grow or wait without limit", NFR-002 lines 19-20) and reported with the FR-061 `limit` code ("a bound is reached; names the bound", FR-061 line 386); the number is an implementation choice and is documented (docs/cli-reference.md:651). Residual, not blocking: the boundary itself has no test (file.rs:75-88 is three lines using `take(MAX+1)`). (2) Transfer to the current owner: FR-061 has the epoch increase by exactly 1 on every claim or transfer (line 180) and a transfer change only the owner record (lines 218-221), so a self-transfer would raise the epoch and change no owner; refusing it with `invalid_argument` (a malformed input) is a defensible reading with no blocking alternative, is asserted (tests/named_focus.rs:1070-1075) and documented (cli-reference.md:681). (3) Claim by the current owner is directly owned by FR-061 ("claim ... SHALL NOT compare the caller's `owner_id` with the stored one", FR-061 lines 196-197); claim of an archived definition is owned by omission in FR-061's exhaustive list of what an archived definition refuses (lines 253-256: update, activation, adoption, transfer), is now tested (tests/named_focus.rs:2340) and documented (cli-reference.md:681). If the owner of the FR later wants it refused, that is a one-clause edit and a one-line code change, not a blocker now. (4) One name in several teams: FR-061's `invalid_argument` is defined as "a malformed input, a name where an ID is required ..." (line 393), which is this case; asserted at tests/named_focus.rs:533-537 and documented (cli-reference.md:647). (5) 128-field record cap: owned by NFR-002 as a bounded read, and its result (`unavailable_definition`, never overwritten) is the FR's own AC-18 behavior for an undecodable record; tested (tests/named_focus.rs:2507) and documented (cli-reference.md:670). No blocking contract decision remains and no spec wave is needed. |

## Current verdict after disposition pass 1

**PASS** for the gap-analysis method, for FR-061 only (planless; no plan completion is claimed). This replaces the round-0 verdict above (CONDITIONAL): of the 9 original findings (3 medium, 6 low), 8 are fixed and 1 (FND-006, low) is accepted-no-change with the reasons above, which I approve; none is open, deferred or rejected, and no new gap was found. FR-062, FR-063 and FR-064 remain out of scope.

Evidence and its limits. All 22 criteria still carry a `#[trace]` tag (I re-read the tags of the new tests; the new tests add tags for AC-1, 3, 4, 7, 8, 9, 11, 12, 16, 17, 18, 19, 20, 21 and 22). I did not recompute the quire matrix: `code-fix1-matrix.json` is one long line my read tool omits, so I make no engine claim about it. A tag shows traceability and not strength; I read the test bodies for FND-003 and FND-004 of SR-114 and the fixtures below, and only the headers and tags for the rest of the new tests listed above, so the "fixed" outcomes for FND-001 to FND-005 and FND-007 are test-presence and branch-mapping judgements, not a whole-assertion audit. I ran no test. The saved final logs show 45 store tests, 9 plus 1 process tests, 25 relay-tui bin tests and the legacy-bytes test passing; the relay-core lib run (179 passed, 1 failed) and the focuses run (31 passed, 1 failed) each ended RED on one test that was a fixture error, corrected afterwards (see SR-114 "Fix review notes"), and only those two tests were re-run alone and passed. A whole-suite green on f813 for the relay-core lib and the agent-msg bin is not in the logs I read; the parent's final aggregate must supply it.

## New findings (disposition pass 1)

None. The two test-only fixture corrections were inspected for assertion weakening and neither weakens one: the blank-name case now covers the two names that are blank under FR-061's sanitizing rule and keeps the `invalid_argument` and empty-store assertions, and the timeout case asserts one more thing (a source read really happened) than before. The tag additions for AC-22 do not change test bodies.
