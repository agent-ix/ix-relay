---
id: SR-6109
title: "Code review — ix-relay SWM-259 FR-072 ordered per-lane assignment list"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/relay-core/src/assignment.rs, crates/relay-core/src/store/assignment.rs, crates/relay-core/src/store/lua/assign*.lua, crates/relay-core/src/store/{keys,moves,fleet}.rs, crates/relay-core/src/{config,error,lib,store}.rs, crates/agent-msg/src/{assign,main,spool}.rs, crates/relay-core/tests/assignment.rs, crates/agent-msg/tests/assign_cli.rs, spec/functional/FR-072-ordered-lane-assignment-list.md"
review_set: subset
---

## Summary

Ticket: SWM-259. PR agent-ix/ix-relay#59 at `a7b60e3cc6af3a376c52797231e4221617b03472`, reviewed as `git diff origin/main...HEAD` against FR-072 (with its new `order_id` clause and AC-10/AC-11), FR-066, FR-069, FR-070 and FR-043 on main. The rust-review lane is folded into this file.

Evidence gathered (macOS, `CARGO_BUILD_JOBS=4`, target dir inside the worktree, throwaway `redis:7` container):

- `cargo test -p relay-core --test assignment`: 12/12 pass.
- `cargo test -p agent-msg --test assign_cli`: 1/1 pass.
- `cargo test -p relay-core --lib keys`: 6/6 pass.
- `cargo clippy -p relay-core -p agent-msg --all-targets -- -D warnings`: clean.
- `quoin matrix --json`: FR-072-AC-1 to AC-11 are all tagged.

## Verdict

**CHANGES REQUESTED.** Two high findings: a lane's list can split from its lane during a rename or relink, and nothing reports it. A case-only tab rename splits the list every time (FND-001). Any follow that errors, or that finds the new address already holding a list, is discarded with `let _ =` (FND-002). The stored probe defect has no reader (FND-003). Reads do not use the registry spelling the writes use (FND-004).

Checked and clean:

- Atomicity. `assign.lua` checks every key's type first (`-7`, nothing written). It then applies the list change, the meta `version`/`at_ms`, the lane-set membership and the transition `XADD` in one script, so a write and its record cannot part. The stale refusal records its defect inside the same script and writes nothing else. `assign_move.lua` renames list and meta, bumps the version and logs the `move` transition in one script as well.
- Bounds. 50 tickets, 64-byte id, 512-byte ref, 64-byte order id and 128-byte writer are validated in Rust (`text()`: non-blank, length, `sanitise_line` identity, so control characters are refused). The ticket count is checked again in Lua against the held list for `append`. Duplicate ids are refused in Rust for `set`/`append`/`reorder` and against held ids in Lua. History: `XADD MAXLEN = 4096`, `XTRIM MINID` at seven days, `EXPIRE` renewed per write, an exclusive `(` cursor, `truncated` and `available_from_ms` as in FR-066. The history limit is 1..1000.
- `not_orchestrator` guard. It is a plain boolean from `[assign] orchestrator` (absent means false, a non-boolean is an error), and it is documented in both the spec and the code as a guard against accidental writes, not authentication. The own-lane check uses `Address` equality, which folds case, so a differently cased own address is still refused.
- `host_registry_stale`. It applies only when every registry entry for the address is an expired host list. A verified registration keeps a lane live, per FR-069. The refusal leaves list, meta and history untouched (tested), and `publish_lane_list` clears the host's defect.
- Read paths. `READ_ONE`, `READ_ALL` and `READ_EVENTS` only call `HMGET`, `LRANGE`, `SMEMBERS`, `XRANGE`, `GET` and `TIME`. `show` and `history` never call `writer()`. TC FR-072-AC-6 compares `DUMP` of every key before and after two rounds of reads.
- Slim EVALSHA scripts follow #56. `CachedScript::new(slim_script(..))` is held in a `LazyLock`. It sends `EVALSHA` first and falls back to `EVAL` on `NOSCRIPT`, and the shared `assign_common.lua` is concatenated before `stripped()` removes comments and indentation, the same way `activity.lua` and `order_expire.lua` are built.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | high | A case-only rename splits the list. `Place` equality is exact, so `refresh_place` swaps and calls the follow. `assignment_follow` then returns `Ok(false)` because `Address` equality folds case (`from == to`). The list stays under the old spelling's Redis key, while every later write uses the registry's new spelling and creates a second list. | crates/relay-core/src/store/assignment.rs:207-211; crates/relay-core/src/store/moves.rs:176-209 |
| FND-002 | high | A failed follow is silent and can split or swap lists. `follow_assignments` discards the result with `let _ =`. A Redis error, or `-8` (the new address already holds a list), leaves the list under the old address with no transition, log, defect or return value. The doc comment's recovery claim is false: a later write to the new address makes a second list, and two lanes that swap tab names silently swap lists. | crates/relay-core/src/store/assignment.rs:402-419; crates/relay-core/src/store/lua/assign_move.lua:14 |
| FND-003 | medium | The `host_registry_stale` probe defect is written to `assign-defects` but nothing reads it. No doctor check, watcher series or alert uses `assignment_probe_defects()`, so the FR-070 defect that FR-072 says SHALL be raised never reaches anyone. | crates/relay-core/src/store/assignment.rs:240-260; crates/relay-core/src/store/lua/assign.lua:15-20 |
| FND-004 | medium | Reads are not resolved to the registry spelling. Writes resolve the address through `registry_lane`, but `assignment(lane)` builds the key from the caller's spelling, so `assign show DRACO:swm:CODE` returns `null` while the list exists under `draco:SWM:code`. A consumer that reads by its own spelling of the address sees no next ticket. | crates/relay-core/src/store/assignment.rs:167-179 |
| FND-005 | low | `registry_lane` matches with `eq_ignore_ascii_case`, but `Address` folds Unicode case (`char::to_lowercase`). A non-ASCII tab spelled differently is equal as an `Address` but is refused as `unknown_lane`. | crates/relay-core/src/store/assignment.rs:157; crates/relay-core/src/address.rs:192-198 |
| FND-006 | low | `assign-lanes` only shrinks on a move. Lists and meta have no TTL, and there is no delete operation (`set` with no tickets keeps the lane in the set). Once 4,096 distinct lanes have ever held a list, every new lane is refused with `lane_limit` for good, and only manual Redis edits can fix it. | crates/relay-core/src/store/lua/assign.lua:23-25,62-64 |
| FND-007 | low | `assign` discards the version the script returns and re-reads the list in a second round trip. A concurrent writer can change it in between, so the returned list and version can belong to another write. | crates/relay-core/src/store/assignment.rs:88,117-121 |
| FND-008 | low | The role and own-lane derivation (`[assign]` role plus the caller's verified place) lives only in agent-msg's private `writer()`. ix-board `fleet assign` (SWM-211), which calls `Relay::assign` directly, has to reimplement it or pass `orchestrator: true`, which would make the guard a no-op on that surface. | crates/agent-msg/src/assign.rs:150-179; crates/relay-core/src/assignment.rs:64-74 |

### Detail

**FND-001.** Trace: the tab `EA` is renamed `ea`. `reg.place != Some(place)` (derived `PartialEq` on `Place`), so `swap_recorded` runs, `swapped == true`, and `follow_assignments(old, new)` calls `assignment_follow("n:EA:code", "n:ea:code")`. `lane_address` parses both, and `Address::eq` uses `eq_fold`, so `from == to` and the function returns `Ok(false)` without running the script. The keys `…assign-list:n:EA:code` and `…assign-list:n:ea:code` are different Redis keys. `registry_lane` now resolves the lane to the registration's new spelling `ea`, so the next `set`/`append` writes a fresh list there. `assignments()` lists both. Fix: compare the exact strings (`from.to_string() == to.to_string()`) before skipping, and add a test with a case-only rename.

**FND-002.** The brief asks whether a silent failure can split a lane's list. It can, in three ways. (a) A Redis error in the follow after the registration swap: the list stays at the old key, and the old address is no longer in the registry (unknown_lane), so the orchestrator writes to the new address and creates a second list. (b) `-8`: the new address already has a list. This happens after any earlier write to that address, including writes made after case (a). (c) Lanes A and B swap tab names: each follow returns `-8`, so each lane now reads the other's list. ix-board and the minder would push the wrong lane's next ticket, which is the outcome this feature exists to prevent. Keeping the move best-effort is acceptable, but the failure must surface. For example: record a `move_failed` transition naming both addresses and the reason, or add the old address to a defects record that `agent-msg doctor` reports. Also return the outcome to `refresh_place`/`relink_pane` callers so the watcher can log it. Add tests for the `-8` and error paths.

**FND-003.** On whether the stored, unread defect needs a doctor warning now or a ticket: both. The doctor warning belongs in this PR, because `agent-msg doctor` is in this repo and lane, already reports fleet warnings (`fleet_lanes_dropped`), and only needs `relay.assignment_probe_defects()`. That makes the refusal visible to operators today. Exporting it as the FR-070 probe series (`ix_fleet_probe_ok{host,source}`, so the `ProbeFailing` alert fires) is relay-watcher work for another lane. File it as a ticket that blocks FR-072-AC-9 from counting as done. Until then a stored defect that nobody reads is the silent failure the spec forbids.

**FND-004.** `registry_lane` already holds the canonical spelling. Reads can resolve it the same way: look the lane up in the registry or in `assign-lanes`, case-folded, and fall back to the given spelling. The test `the_registry_spelling_of_an_address_names_the_list` should also read back by the non-canonical spelling.

**FND-008.** Move a `AssignWriter::from_config(relay, machine, &FileConfig)` (or similar) into relay-core so both CLIs share one derivation.

## Dispositions

Round 1. The fix head is `602d11a861f29d9be1765beac3e9c637a94bf644` (`a7b60e3..602d11a`).

Evidence (macOS, redis:7):
- relay-core `--test assignment`: 19/19 pass, 7 of them new.
- agent-msg `assign_cli`: 1/1 pass.
- The doctor test `tc_072_12`: passes.
- `cargo clippy -p relay-core -p agent-msg --all-targets -D warnings`: clean.
- `cargo fmt --check`: clean.

Merge status: the branch no longer merges into origin/main (`c7b3c6a`, Release 0.4.0, #60). `git merge-tree` reports `CONFLICT (content)` in CHANGELOG.md, and GitHub reports `mergeable: CONFLICTING`.

| FND | outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | `assignment_follow` now compares the address texts (`from.to_string() == to.to_string()`); test `a_case_only_rename_moves_the_list`. |
| FND-002 | fixed | The follow returns `FollowOutcome`. `-8` and store failures record a `move_failed` transition and a `MoveDefect`, are logged by `follow_assignments`, and are reported by doctor. Tests cover an occupied target, a store failure and a swap. The settlement rule has a new defect (FND-009). |
| FND-003 | fixed | Doctor reports `assignment_host_registry_stale` and `assignment_move_failed` in an Assignments family, and `assignments_unreadable` as unknown; test `tc_072_12`. The FR-070 series export is still open, as SR-6110 FND-001. |
| FND-004 | fixed | `assignment()` resolves the address through `registry_lane` before reading; test `any_spelling_of_an_address_reads_and_writes_the_one_list`. |
| FND-005 | fixed | `registry_lane` now compares with `Address` equality (Unicode case folding); test `addresses_fold_case_beyond_ascii_as_address_equality_does`. |
| FND-006 | fixed | An emptied list leaves `assign-lanes`. When the lane limit is hit, `prune_assignments` drops lists of lanes no registry lists that are older than seven days, then the write is retried once. The lead accepted pruning only at the limit. |
| FND-007 | fixed | The write script returns `{status, version, ms, entries}` and `assign` builds the list from that reply, with no re-read. |
| FND-008 | fixed | `AssignWriter::from_config` in relay-core derives the role and writer name for every caller; `Relay::assign` keeps the guard. The caller's own lane is still supplied by the caller, because it is the caller's identity. |

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-009 | high | `assign_settle` ends every defect that names the written lane on either side. After a swap (two defects, `code>review` and `review>code`), a write that corrects one lane also clears the defect of the other lane, which still reads the wrong list. A later successful move through either address clears both as well. | crates/relay-core/src/store/lua/assign_common.lua:28-35; crates/relay-core/src/store/lua/assign.lua:76; crates/relay-core/src/store/lua/assign_move.lua:39-40 |
| FND-010 | low | The move-defects hash gets `EXPIRE` of seven days on each new defect. An unsettled defect therefore vanishes from doctor after seven days even though the list is still under the old address. | crates/relay-core/src/store/lua/assign_common.lua:45-46 |
| FND-011 | medium | The PR conflicts with origin/main after the 0.4.0 release (#60): CHANGELOG.md has a content conflict with the new `## 0.4.0` section, and GitHub reports CONFLICTING. Rebase and keep the FR-072 entry under `## Unreleased`. | CHANGELOG.md:3-5 |

### Detail (disposition pass 1)

**FND-009.** Trace: lane A is at `code` with list [A-1] and lane B is at `review` with [B-2]. They swap tab names. Both follows return `-8`, which records the defects `code>review` and `review>code`. Lane A (now at `review`) reads [B-2], and lane B (now at `code`) reads [A-1], so both are wrong and both are reported. The orchestrator then runs `assign set terra:SWM:review ...` to fix lane A. `assign_settle(review)` deletes both defects because each one names `review`. Lane B still reads A's list at `code`, and doctor no longer warns about it. The minder pushes A's tickets to B.

Fix: a write settles only the defects whose `to` is the written lane, since that is the lane whose current list the orchestrator just wrote. A successful move settles only the defects whose `to` is the move's `from`, if the lead keeps that rule at all. Extend `two_lanes_that_swap_tab_names_keep_their_lists_and_raise_the_defect`: write one lane, and assert that the other lane's defect still stands.

Round 2. The head is `79d1d6b9b0dcf06d8d6400fd62b1abb71964acdc`: one commit, rebased onto `c7b3c6a`. Its content was compared with `602d11a`.

Evidence (redis:7):
- relay-core `--test assignment`: 20/20 pass.
- agent-msg `assign_cli`: 1/1 pass.
- Doctor `tc_072_12`: passes.
- clippy -D warnings and fmt: clean.

Merge status: origin/main has moved to `e9e256c` (#58, SWM-238), which adds its own `## Unreleased` CHANGELOG entry. `git merge-tree` reports `CONFLICT (content)` in CHANGELOG.md, and GitHub reports `mergeable: CONFLICTING`.

| FND | outcome | reason |
| --- | --- | --- |
| FND-009 | fixed | `assign_settle` now ends only the defects whose `to` is the written lane. A successful move ends only its own `from>to` pair. The swap test now writes one lane and asserts that the other lane's defect still stands, including after an unrelated move. |
| FND-010 | fixed | The defects hash has no `EXPIRE`. `prune_assignments` drops a defect only once no list is left under its `from` address. Test `a_move_defect_never_expires_while_the_list_is_still_under_the_old_address` asserts TTL -1 and that the defect survives a prune while the split stands. |
| FND-011 | still-open | The rebase onto `c7b3c6a` resolved the 0.4.0 conflict, but main has since taken `e9e256c` (#58), and CHANGELOG.md conflicts again under `## Unreleased`. Rebase onto `e9e256c` and merge both Unreleased entries. |
