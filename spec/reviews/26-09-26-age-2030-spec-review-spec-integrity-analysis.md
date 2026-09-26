---
id: SR-009
title: "Integrity of FR-044, the README and the test matrix at ix-relay#26 head 36c2c46"
type: SpecReview
analysis: integrity
scope: "agent-ix/ix-relay@36c2c46626cdf092005a5a05bd525f7a9780e29b; spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md, spec/spec.md, spec/tests.md (FR-044 and TC-044 rows), README.md (socket delivery section), plugins/claude/README.md"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-044"
    type: "reviews"
---
# SR-009: Integrity of FR-044, the README and the matrix (round 3)

## Summary

Ticket: AGE-2030. PR: ix-relay#26 at 36c2c46. Checked: consistency between FR-044, the
README sections and the implementation after the round-3 edits, AC atomicity, and the
honesty of the verification statements.

Traceability holds (US-002 → FR-044 → AC-1..9 → TC-044-01..09, and `make check-matrix`
reports 299 rows OK). The verification wording is honest. AC-3 and wire.rs claim only
`held` and `refused` as captured live from 2.1.283, and mark `delivered`, `denied`,
`dropped`, `hold_timeout` and the duplicate detail as fixtures. The round-3 live check of
the unconfirmed-post path is claimed only in the PR thread, not in the repo.
plugins/claude/README.md is consistent with FR-044. Five inconsistencies remain, all low.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | The README says "Decisions about a batch are retried until their write to the bus succeeds". FR-044 and the code give up after five failed tries, after which the batch is redelivered (SR-006 FND-003). The README overstates the guarantee. | README.md (Delivery through Claude Code's inbox socket), spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md |
| FND-002 | low | FR-044 says the status file is kept "in a 0700 directory". The implementation only creates a missing dir with 0700; the existing shared state dir keeps its mode (SR-006 FND-008). | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md, crates/relay-core/src/socket_status.rs:168-174 |
| FND-003 | low | FR-044 says both "the watcher shall keep the held batches in its status file, so that a restarted watcher adopts them" and "shall not dead-letter an approved message as never approved". The status record has no approved or delivered state, so the two conflict across a restart (SR-006 FND-004). | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md, crates/relay-core/src/socket_status.rs:83-94 |
| FND-004 | low | FR-044 states a "6 s socket budget" that keeps many wedged leads from holding a tick past the heartbeat's expiry, and separately "a connect gives up after 2 s and a write after 2 s". The code docs call the budget "the most time one tick spends on sockets". Because the budget is checked before the connect and the write time is uncounted, the bounds add up to about 10 s, not 6 (SR-006 FND-005). State the composed bound, or count connect and write against the budget. | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md, crates/relay-watcher/src/socket/deliverer.rs:127-131 |
| FND-005 | low | FR-044-AC-5 and FR-044-AC-6 have each grown in round 3 to more than a dozen independent conditions in one criterion. A failure cannot name which obligation broke, and the TC rows list 33 and 15 tests against a single AC. The pattern predates round 3 but has grown. Consider splitting AC-5 (hold and renewal, late receipts, settlement retry, grace, adoption) and AC-6 (fallback, budget and starvation). | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (Acceptance Criteria), spec/tests.md |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-006 | low | The AC split dropped two obligations that the Behavior section and the matrix still carry. AC-8 now says only "the reply socket is removed on SIGTERM", but TC-044-08 says "the reply socket and status file are removed on SIGTERM" and lists `tc_044_08_the_status_file_is_kept_at_exit_only_for_a_held_batch`, whose obligation (the file is removed at exit unless a held batch keeps it) is in no AC. AC-9 lost "a batch dead-lettered as `denied` or `hold_expired` is shown with its reason for 24 hours" (AC-15 only covers pruning), while TC-044-09 still lists `tc_044_09_a_denied_or_expired_dead_letter_is_reported_with_its_reason`. Put both back in an AC. | spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (FR-044-AC-8, FR-044-AC-9, FR-044-AC-15), spec/tests.md (TC-044-08, TC-044-09) |

## New findings (disposition pass 2)

Checked at dc4ccd52dd29a424c73ce3defc81b7a2fa26eba5. `make check-matrix`: 306 rows, OK. Every
`tc_044_*` name in spec/tests.md is defined, every defined one is listed, and the new tests carry
matching `Tracing:`/`ACs:` tags. `quire validate` on FR-044 passes.

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-007 | low | The TC rows' descriptions were not updated with the ACs. TC-044-15 still reads "made 0700 (also when it existed at 0755) ... not read from a group- or world-writable directory", which contradicts FR-044-AC-15 and the code (an existing open directory is now refused and never repaired). TC-044-11, TC-044-13 and TC-044-14 do not mention the obligations added to their ACs (the settlement note and owed decisions, owed-decision validation, postponement and the write deadline). The "dead-letter reasons older than 24 hours are dropped" clause left AC-15. It is now only implied by AC-9's "shown ... for 24 hours", whose TC-044-09 test does not check time. The 24-hour drop is tested by `tc_044_13_adoption_validates_every_field_and_the_caps`, traced to AC-13, whose text does not state it. The same "Fifth-round mutants ..." sentence is pasted into the AC-11, AC-14 and AC-15 rows of both tables. | spec/tests.md (TC-044-11, TC-044-13, TC-044-14, TC-044-15 rows; FR-044-AC-11/14/15 rows), spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md (FR-044-AC-9, FR-044-AC-13, FR-044-AC-15) |

## New findings (disposition pass 3)

Checked at 1df698cd1ba5ffd5919c1d6d2b606d70ae7f8180 (squashed and rebased onto main 2e4bc1d, which
brings FR-045, the OTLP push).

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-008 | low | The status file's path is stale in two places. FR-044, the code and AC-15 keep it at `state/socket/socket-<pid>.json` (a directory of its own since e002abf), but README.md:313 still says "per watcher (`state/socket-<pid>.json`)" and the `socket_status` module doc says it is written to "`~/.ix/relay/state/socket-<watcher pid>.json`". An operator looking for the file, or at its directory's mode, is sent to the wrong place. | README.md:313, crates/relay-core/src/socket_status.rs:1-8 |
| FND-009 | low | FR-044 and FR-045 (arrived with the rebase) disagree on what `ix.relay.messages{outcome=delivered}` counts. FR-045 defines `delivered` as "typed into a pane by this watcher" and `requeued` as "put back by a rejected prompt". A socket post that is confirmed at once returns `Outcome::Delivered` and is counted as `delivered` although nothing was typed; a held message approved later is acknowledged in `apply_settlements` and never counted; a socket denial, a postponement and a deferral are counted nowhere. Neither FR states how socket outcomes map to the metric. State the mapping in FR-044 (or widen FR-045's definitions), and count the late acknowledgement if `delivered` is meant to cover the socket. | spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:178, crates/relay-watcher/src/main.rs:920-924, crates/relay-watcher/src/main.rs:980-1045, crates/relay-watcher/src/watch.rs:345-370 |

## Dispositions

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed | e002abf: README now says a decision is kept until its write succeeds, renewed and retried with a growing wait, alerted after five failures and never given up on. |
| FND-002 | fixed | e002abf: the status files are in `state/socket`, created 0700, an existing one is made 0700 before writing (code and test agree with FR-044 and the README). |
| FND-003 | fixed | e002abf: FR-044 keeps the held batches "and when each was reported delivered" in the status file; `HeldBatch.delivered_since_ms` implements it. |
| FND-004 | fixed | e002abf: FR-044 states the 8 s budget, the 7 s worst case of a post and what is counted; the code doc names the uncounted transcript scan. The per-call write bound is SR-006 FND-011. |
| FND-005 | fixed | e002abf: AC-5 and AC-6 are split into AC-5, AC-6 and AC-10..AC-16, with TC-044-10..16 rows. |
| FND-006 | still-open | New this round; not addressed. Two obligations dropped from the ACs by the split. |
| FND-006 | fixed | dc4ccd5: FR-044-AC-8 again states "the reply socket and status file are removed on SIGTERM (the file kept only while a held batch or an owed decision must survive)", and FR-044-AC-9 states "a batch dead-lettered as `denied` or `hold_expired` is shown with its reason for 24 hours". TC-044-08 lists `tc_044_08_the_status_file_is_kept_at_exit_only_for_a_held_batch`, and TC-044-09 lists `tc_044_09_a_denied_or_expired_dead_letter_is_reported_with_its_reason`. Both pass, and both tags match. |
| FND-007 | still-open | New this round; not addressed. TC descriptions are stale against the ACs, and the 24-hour drop is traced to an AC that does not state it. |
| FND-007 | fixed | 1df698c (squashed; checked at that head): TC-044-15 now reads "an existing directory that is open, a symlink or another user's is refused and never chmodded"; TC-044-11, -13 and -14 name the settlement note and owed decisions, owed-decision validation and the popper guards, and the postponement and write deadline; FR-044-AC-13 states "dead-letter reasons older than 24 hours are dropped on adoption"; the pasted "Fifth-round mutants" sentence is gone. `make check-matrix`: 322 rows OK; no duplicate TC or AC row after the rebase onto the FR-045 rows. |
