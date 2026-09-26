---
id: SR-006
title: "Code review (with Rust lane) of ix-relay#26 at 36c2c46: socket delivery, fourth pass"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@36c2c46626cdf092005a5a05bd525f7a9780e29b; crates/relay-watcher/src/socket/deliverer.rs, crates/relay-watcher/src/socket/transcript.rs, crates/relay-watcher/src/socket/session.rs, crates/relay-watcher/src/socket/wire.rs, crates/relay-watcher/src/socket.rs, crates/relay-watcher/src/socket/tests.rs, crates/relay-watcher/src/main.rs, crates/relay-watcher/src/watch.rs, crates/relay-watcher/src/episodes.rs, crates/relay-watcher/src/supervise.rs, crates/relay-watcher/src/deliver.rs, crates/relay-core/src/socket_status.rs, crates/relay-core/src/store/inflight.rs, crates/relay-core/src/store/lua/renew.lua, crates/relay-core/src/store/lua/deadletter.lua, crates/relay-core/src/config.rs, crates/agent-msg/src/doctor/checks.rs"
review_set: subset
relationships:
  - target: "ix://agent-ix/ix-relay/FR-044"
    type: "reviews"
---
# SR-006: Code review (with Rust lane) of ix-relay#26 at 36c2c46

## Summary

Ticket: AGE-2030. PR: ix-relay#26, head 36c2c46 (the third-round fix commit on top of
c8b195c). This pass attacked the round-3 fixes: the tick-budget redesign, retried
settlement writes, the `delivered` grace, checked adoption of a dead watcher's status file,
the status file's mode, the dead-letter reason expiry, the SIGTERM EINTR guard and the
bounded transcript catch-up. The Rust lane (rust-review) is folded in here.

Method: a read-only review of the worktree. Probe tests ran from a scratch copy of the
source (`git archive HEAD`), built into the worktree's own target dir, against a throwaway
`redis:7` on 127.0.0.1:16399 that is stopped afterwards. Only fake Claude Code receivers
(temp registry, temp projects, sockets bound by the test) were used. No real session,
socket, registry record or transcript was read or touched, and no `claude -p` worker was
started. The full `make ci` gate was not re-run (the lead runs it on this head).

Most of the round-3 work holds. Two round-3 changes introduce new defects (FND-001 and
FND-002, both medium, both small to fix). The rest are low.

### Round-3 fixes re-verified at this head

| Round-3 item | Result at 36c2c46 |
|---|---|
| R3-1 (high) budget below the ack wait | Fixed. `TICK_BUDGET` 6 s, `DEFAULT_ACK_WAIT` 3 s, const assert at deliverer.rs:135; post only when `tick_spent + ack_wait <= tick_budget` (deliverer.rs:662); a whole-wait miss marks the lead suspect (deliverer.rs:799). The production-constant tests exist. A probe with 20 silent receivers and one healthy one: every tick took about 3.1 s (one post each), and the healthy lead was served on the 21st tick. |
| R3-2 (med) settlements dropped before the write | Fixed. `Resolution` stays on `Pending` until `applied` reports `Ok` (deliverer.rs:932-994). A connection error leaves later settlements owed (main.rs `apply_settlements`). What happens after the fifth failure is FND-003. |
| R3-3 (med) false notification on a budget cut | Fixed for the notification: a budget cut is `Deferred`. But the new mapping swallows the bypass notice (FND-002). |
| R3-4 (low) approved message dead-lettered as never approved | Fixed within one watcher's life (5-minute grace, `delivered_unconfirmed`). Not across a restart (FND-004). |
| R3-5 (low) adoption trusted the file | Fixed. Fields are validated (deliverer.rs:449-487). `renew.lua` and `deadletter.lua` match batch id and popper on the lead's own key. Forged `since_ms: 0` is rejected. See FND-007 for the arithmetic. |
| R3-6 (low) file mode | The file is 0600 and fsynced before the rename. "0700 dir" holds only for a newly created dir (FND-008). |
| R3-7 (low) reasons never expire | Fixed. `prune_dead_notes` runs at start, at settle and on each note, and `Drop` keeps the file only for a held batch. |
| R3-8 (low) SIGTERM EINTR logged | Fixed. `Supervisor::stop_on` plus the `is_interrupted` guard in `on_failure`, and `run_until` breaks when stopping. The module doc names the signal exit. |
| R3-9..11 (docs) | Fixed in FR-044 and the README: the lost-renewal lead stays held, the >120 s gap is at-least-once, ordering is stated, and the Dependencies line is corrected. |
| R3-12 (test) | Fixed. `says_gone == 1` (tests.rs:2482), and production-ratio tests were added. |
| R3-13 (info) settle cost and >512 KiB gap | Fixed. Settle time counts toward the budget, and `Watch` catches up in 512 KiB windows. The same change introduced FND-001. |

### Rounds 1 and 2 (posted on the PR before the reviewer skill), re-checked at this head

Checked by reading the code and the tests at this head, not re-probed. Round 1: #1 (held
quiet) fixed (`holds` pre-pop plus unmarked `Deferred` requeue). #2 (late receipts) fixed
(receipt thread plus `settle`). #3 (ambiguous never falls back) fixed (`suspect`). #4 (bind
once) fixed (`ensure_bound` backoff). #5 and round 2 #11 (escape) fixed: every `<` is
written `&lt;` (wire.rs:80-87). #6 (unlink a live socket) fixed (`probe_is_live`). #7
(socket dir, owner, mode) fixed (session.rs:264-272). #8 and #9 (test gaps) fixed. #10
(tick cost) superseded by the budget. #11 (SIGTERM) fixed. #12 and #13 (doctor text,
unknown on config failure) are tested. #14 (backticks) fixed: no doubled backticks remain.
#15 (one status file) fixed (per pid). Round 2: #1 (lease renewal) fixed (`renew.lua`). #2
(pre-pop check pinned) fixed (`tc_044_05_a_lead_the_transport_holds_has_nothing_popped`).
#3 (SIGTERM docs) fixed. #4 (statuses labelled measured) fixed (AC-3, wire.rs:96-99). #5
(denied reason) fixed (`dead_notes`). #6 (Lua type guard test) fixed. #7 (in-wait denial)
fixed (`Denied`). #8 (tick cost) superseded by the budget. #9 (pid reuse) fixed (start
time). #10 (probe flag) fixed. #13 (truncated transcript) handled (tail on shrink).

### Also checked, no finding

- Default off: `delivery()` returns `herdr_only` for `DeliveryMode::Herdr` without building
  a `SocketConfig`, binding or writing. `delivery` is a raw tolerated `toml::Value` that
  only `delivery_from_file` validates.
- `renew.lua` and `deadletter.lua` declare their keys, check types before any write, match
  id and popper, and never shorten a lease.
- No `unwrap`/`expect` outside test modules in the changed production files, no new
  `allow`, no `.github` change, no `unsafe`, and no writes to Claude Code files.
- Verification wording in AC-3, wire.rs and the README is honest. Only `held` and `refused`
  are claimed live. The unconfirmed-post scratch check is claimed only in the PR thread,
  not in the repo.
- Observed, not a finding: `MAX_PENDING` eviction (deliverer.rs:699-708) skips held entries
  but not entries that still owe a `Resolution`. Reaching 256 open non-held sends is not
  realistic at the swarm's size.

## Verdict

CONDITIONAL. Not ready to merge: FND-001 (a double-delivery path) and FND-002 (a lost
bypass-mode notification) are round-3 regressions. Both are small to fix. The low findings
can be fixed or deferred with reasons.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | medium | Round-3 regression: the transcript baseline is taken after the post. `attempt` writes and flushes the line, then runs `transcript::find` (a `read_dir` of the whole projects dir) and `Watch::new`, which records the file's length at that moment as `seen_to`. A receiver that appends the `origin.msg_id` line before that baseline is never seen. The ack wait ends with neither a receipt nor a transcript line, so the delivered batch is `Ambiguous`: the lead is marked suspect for 10 min, the operator gets "may have failed", and the batch is redelivered, marked, through herdr, which is a duplicate. c8b195c's `Watch::new` read the tail on its first look, so it did not have this. Probe (a fake receiver that records as soon as it reads, transcript already present): 1/20 judged ambiguous with no extra project dirs, 2/20 with 20, 15/20 with 200. None was ever acked later. The real 2.1.283 session recorded after about 130 ms (tests.md AC-4 row), which narrows the window but does not close it on a loaded machine. Fix: build the `Watch` (the file length) before `write_all`. | crates/relay-watcher/src/socket/deliverer.rs:689-728, crates/relay-watcher/src/socket/transcript.rs:68-71 |
| FND-002 | medium | Round-3 regression: a budget deferral swallows the bypass-mode notification. A budget cut returns `SocketError::Deferred` and becomes `Outcome::SocketDeferred`, whose `episode()` is `EpisodeKind::SocketHeld`. `Episodes::observe` then inserts the lead into `notified` although nobody was told. If the next tick's post to that lead is held (bypass mode), `notice` is `Done`, so `notified: false` and no notification is sent. The operator first hears when `hold_expired` fires 30 min later. It also logs "mail held for approval; put N message(s) back" and opens a "mail held by Claude Code for approval" episode for a lead that was only deferred. It needs a wedged or unconfirmable lead (e.g. another `CLAUDE_CONFIG_DIR`, which recurs every 10 min) to use the budget in the tick before. Probe with watch.rs fakes: tick 1 deferred, tick 2 `SocketHeld { notified: false }`, notifies = 0. Fix: give a budget deferral its own outcome with no episode (or not `SocketHeld`), and only mark `notified` when a notification was attempted. | crates/relay-watcher/src/watch.rs:166-177, crates/relay-watcher/src/watch.rs:215-217, crates/relay-watcher/src/episodes.rs:124-127, crates/relay-watcher/src/socket/deliverer.rs:661-667 |
| FND-003 | low | After `MAX_SETTLE_TRIES` (5) failed writes the pending entry is dropped, so renewals stop too. The lease (at most 120 s) lapses, the reaper returns the batch marked, and it is posted again. A denied message is re-prompted and a failed ack becomes a duplicate. Connection errors at the settlement step also count toward the 5. Probe (Redis, dead-letter key broken until after give-up, then the lease lapses): reaped `requeued: 1`, then socket posts = 2 and notifications = 2 for a message the user denied. The code's log line says so, but FR-044 does not say what follows `settlement_failed`, and the README says decisions "are retried until their write to the bus succeeds". Either keep renewing a held batch whose decision is owed (with backoff) or state the at-least-once consequence in FR-044 and the README. | crates/relay-watcher/src/socket/deliverer.rs:975-990, crates/relay-watcher/src/main.rs:952-1012, README.md (socket section) |
| FND-004 | low | `HeldBatch` in the status file carries no `delivered_at`. A batch that Claude Code reported delivered (approved) but whose transcript line has not shown yet is adopted by a restarted watcher as a plain hold. At `MAX_HOLD` it is dead-lettered `hold_expired` with the "was never approved" notification, which breaks FR-044's "shall not dead-letter an approved message as never approved". It needs a restart inside the 5-minute grace of an unconfirmable delivery. Probe: held → `delivered` → restart (dead-pid status file) → `DeadLetter` "hold_expired … never approved". Fix: persist a delivered flag or time and apply the grace-ack on adoption. | crates/relay-core/src/socket_status.rs:83-94, crates/relay-watcher/src/socket/deliverer.rs:449-487, crates/relay-watcher/src/socket/deliverer.rs:1101-1116 |
| FND-005 | low | The tick budget is checked before the connect, so it is not a ceiling. A post allowed at `tick_spent` just under 3 s can still spend up to 2 s connecting (counted only afterwards), up to 2 s writing (never counted) and 3 s waiting, and `session::resolve` and `transcript::find` are never counted. One tick's socket time can reach about 10 s against the documented "the most time one tick spends on sockets" of 6 s. That is still well inside the 45 s heartbeat TTL, so this is a documentation or accounting issue, not a wedge. | crates/relay-watcher/src/socket/deliverer.rs:127-131, crates/relay-watcher/src/socket/deliverer.rs:661-698 |
| FND-006 | low | A healthy lead ordered after N wedged leads waits N ticks. Probe at production constants: 20 silent receivers plus 1 healthy one last, each tick about 3.1 s, and the healthy lead was served on the 21st tick. Lead order is fixed (registration order), so the same leads always wait. If the wedged leads keep getting mail and N × the tick interval reaches `SUSPECT_FOR` (10 min), some wedged lead leaves suspicion on every tick and the leads after it are deferred indefinitely. The AC-6 test only shows that tick 2 reaches lead 2. Consider rotating the start lead, or letting a lead that is only deferred go first next tick. | crates/relay-watcher/src/socket/deliverer.rs:661-667, crates/relay-watcher/src/main.rs:781-900, crates/relay-watcher/src/socket/tests.rs:2615-2666 |
| FND-007 | low | Rust lane: the times from another process's status file use unchecked `i64` subtraction: `now - n.at_ms` in `prune_dead_notes` and `now - h.since_ms` in adoption. A value of `i64::MIN` panics in a debug build. Probe: "attempt to subtract with overflow" at deliverer.rs:390 on start. In release it wraps, so a forged `at_ms: i64::MIN` note is kept forever (the wrapped age is negative and passes `<= age_ms`). The doctor side already uses `saturating_sub` for the same kind of value. Use `checked_sub` and treat `None` as invalid. | crates/relay-watcher/src/socket/deliverer.rs:386-394, crates/relay-watcher/src/socket/deliverer.rs:450-451 |
| FND-008 | low | "In a 0700 directory" holds only when the status write creates the directory. `DirBuilder::mode(0o700)` does not change an existing dir, and the state dir is shared with `watcher.json`, which `state_file.rs` creates with `create_dir_all` at the umask default. On any existing install the dir stays at, for example, 0775. The file itself is 0600, so the contents are protected; the FR claim is not true. No test checks the directory mode. Either `set_permissions` the dir or reword FR-044. | crates/relay-core/src/socket_status.rs:168-174, crates/relay-watcher/src/state_file.rs:44, spec/functional/FR-044-watcher-delivers-through-claude-codes-inbox-socket.md |

## New findings (disposition pass 1)

Found while re-verifying the fix commit e002abf (head e002abfb15b106f4ff82424782f44aa3ccf46cee).
Probes ran from a scratch `git archive` copy of e002abf built into the worktree's target dir,
against a throwaway `redis:7` and fake receivers only.

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-009 | low | Introduced by the FND-002 fix: `Outcome::SocketPostponed` maps to no episode, and `Episodes::observe(None)` ends whatever episode the lead is in and clears `notified`. A lead in a `SocketHeld` episode (the operator already notified) whose re-post after the 10-minute quiet period is postponed by the budget logs "mail held by Claude Code for approval ended", and the next tick's post behind the same prompt notifies the operator a second time. Probe with watch.rs fakes, held / postponed / held: transitions `started SocketHeld`, `ended SocketHeld`, `started SocketHeld`; 2 bypass notifications for one prompt. A postponement should leave the episode and `notified` untouched (skip `observe`, or observe the current kind). | crates/relay-watcher/src/watch.rs:181-186, crates/relay-watcher/src/episodes.rs:124-131, crates/relay-watcher/src/main.rs:935, crates/relay-watcher/src/socket/deliverer.rs:726-753 |
| FND-010 | low | The status-directory trust check tests only the mode bits (`mode & 0o022 == 0`), not the owner, and follows symlinks (`fs::metadata`). The watcher's own first write then `chmod`s an open directory to 0700 without looking at what it already holds, which makes files planted while it was open trusted afterwards. Probe: `state` at 0777 with a forged dead watcher's file naming batch 7 of lead `ea`; the first watcher adopts nothing (correct), its publish makes the dir 0700 and leaves the forged file, and the next watcher adopts it and renews batch 7. The bus popper/lead checks still limit the effect to batches this watcher popped. Check `uid == geteuid()` and `symlink_metadata`, and either refuse or clear foreign files before trusting a dir the watcher just tightened. | crates/relay-core/src/socket_status.rs:36-39, crates/relay-core/src/socket_status.rs:51-63, crates/relay-core/src/socket_status.rs:198-208 |
| FND-011 | low | The 2 s write bound is per `write` call (`set_write_timeout`), not per post, so FR-044's "give up ... a write after 2 s" and the new "worst case of a post, 7 s" (the `TICK_BUDGET` doc, README, FR-044) do not hold for a line larger than the socket buffer (lines go up to 1 MiB) sent to a receiver that reads slowly but within 2 s per read. Probe: a 900 KiB batch to a receiver reading 128 KiB every 1.8 s: one post took 14.06 s (budget 8 s). The receiver is a local Claude Code, so this is unlikely; the theoretical bound grows with line size and can approach the 45 s heartbeat TTL. Bound the whole write with a deadline (shrink the timeout per call), or state the per-call bound. | crates/relay-watcher/src/socket/deliverer.rs:774-784, crates/relay-watcher/src/socket/deliverer.rs:127-135 |

Observed, not findings: (a) a `settlement_failed` note lives in `state.notes`, which a later
delivery to the same lead overwrites or clears, so `doctor` can lose it while the write still
fails (the notification is still shown once); (b) `publish` persists pending entries only by
`held_since`, without their owed resolution, so after a restart a denied batch whose dead-letter
write kept failing is adopted as a plain hold and dead-lettered as `hold_expired` ("never
approved"), and an owed transcript-ack of a never-held batch is not persisted at all; both need a
restart during a persistent bus-write failure.

## New findings (disposition pass 2)

Found while re-verifying the fix commit dc4ccd5 (head dc4ccd52dd29a424c73ce3defc81b7a2fa26eba5).
Probes ran from a scratch `git archive` copy of dc4ccd5 built into its own target dir, against a
throwaway `redis:7` and fake receivers only.

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-012 | low | Owed decisions restored from a status file are applied without the popper guard that FR-044-AC-13 relies on ("a batch id that the lead's own hook holds is neither renewed nor dead-lettered"; "owed decisions are validated the same way"). `apply_settlements` calls `relay.ack(&lead, batch)` and `relay.requeue_batch(&lead, batch)`, and `ack.lua` and `requeue.lua` match the batch id only; only `deadletter.lua` and `renew.lua` check the popper. Probe (Redis): the lead's hook pops batch 1, and a dead watcher's file carries one owed entry for batch 1. An owed `ack` deletes the hook's in-flight batch ("done", in flight 1 -> 0). An owed `requeue` puts the hook's batch back, and the watcher then delivers it a second time. An owed `dead_letter` is refused ("no longer in flight"). Batch ids come from one `INCR` counter, so an honest stale entry collides only after the bus's counter is reset (a flushed or replaced Redis between the crash and the restart). A forged one needs this user's 0700 directory. Fix: pass `Popper::Watcher` to ack and requeue, at least for adopted decisions, and match it in the scripts. | crates/relay-watcher/src/main.rs:984-987, crates/relay-core/src/store/lua/ack.lua, crates/relay-core/src/store/lua/requeue.lua, crates/relay-watcher/src/socket/deliverer.rs:549-603 |

The two items observed in pass 1, re-checked at dc4ccd5:

- (a) The `settlement_failed` note overwritten by a later note: resolved. The note now lives in
  `failed_notes`, a map keyed by lead (bounded by the number of leads) that is published beside
  the other notes. `tc_044_11_a_settlement_failed_note_survives_a_later_delivery_note` passes.
  Two remaining points, observed but not findings. Any successful non-renewal write for the lead
  clears the note, even when another batch's write for that lead still fails. The commit message
  says the note "persists across restarts", but it is not adopted. It is re-raised after five
  more failures, with a second notification, because `attempts` and `alerted` restart at 0.
  FR-044 does not claim it persists.
- (b) Owed decisions not persisted across a restart: resolved. `SocketStatus.owed` is written
  with the rest of the status: a temp file with `create_new` and mode 0600, fsync, then rename.
  It is adopted after the held batches, and its fields are validated (action, reason, lead name,
  batch id not 0, checked age within the dead-note age). The probes: 2 x 1000 owed entries restore
  256 (`MAX_PENDING`). A replay whose batch is already gone (`Ok(0)`) is dropped, and the file is
  removed at a clean exit. `Drop` keeps the file while a decision is owed.
  `tc_044_11_owed_decisions_are_persisted_and_applied_after_a_restart` (denial stays `denied`,
  the ack is not lost) and `tc_044_13_owed_decisions_from_a_status_file_are_validated` pass. The
  missing popper guard is FND-012.

Also checked at dc4ccd5, no finding. `cargo test -p relay-watcher -p relay-core -p agent-msg` on
Redis passes (relay-watcher: 262 unit tests), including the round 1-4 regression tests at this
head: baseline before the write (200 rounds, 0 misses), kept settlements (20 ticks, 1 post),
delivered grace across a restart, fair rotation (21 ticks), `i64`-edge times, the `state/socket`
subdirectory and herdr-only default off. Mutants: dropping the postponement skip in
`observe_outcome` fails `tc_044_14_a_postponement_leaves_a_held_episode_untouched`, and a
per-call `write_all` fails `tc_044_14_a_slow_reader_cannot_stretch_the_write_past_its_deadline`
(13.3 s). This round's production diff adds no `unwrap`/`expect`, `allow` or `unsafe`, and the
PR changes nothing under `.github`. A refused status directory still leaves socket delivery
working (the probe delivered and was received). Its status is not persisted, `doctor` shows
one unreadable entry, and each publish logs one "cannot write" line (only on ticks with socket
activity). Observed, not a finding.

## New findings (disposition pass 3)

Checked at 1df698cd1ba5ffd5919c1d6d2b606d70ae7f8180, the PR squashed to one commit and rebased
onto main 2e4bc1d (OTLP push, free-variant fix, in-flight list, doctor/heartbeat, newtypes). The
PR diff (`git diff origin/main...HEAD`) was re-reviewed; the pre-squash fix commit was 12df7e7.
Probes ran from a scratch `git archive` copy built into its own target dir, against a throwaway
`redis:7` and fake receivers only. No new code defect was found in this file's scope.

Rebase checks, no finding. `cargo test --workspace` on Redis passes (41 suites, 0 failures;
relay-watcher 279 unit tests), including `an_idle_tick_is_two_requests_and_no_herdr_call`
(FR-038-AC-5) and both `tc_045_22_*` push-tick pins: `apply_settlements` makes no request when
nothing is owed, and herdr-only delivery owes nothing. The OTLP publish stays last in the tick,
after the socket work, so a wedged socket lead can delay it (and the heartbeat, written after the
tick) by at most the 8 s socket budget plus the uncounted transcript scan, against a heartbeat
TTL of at least 30 s. `delivery` is a raw tolerated value beside the raw OTLP keys: a probe
config with `delivery = "uds"`, `otlp_endpoint = 5` and `otlp_interval_secs = "x"` still loads
for `agent-msg who`, `send` and `hook` (exit 0); `doctor` reports `watcher_otlp_config_invalid`
(warn) and `socket_status_unknown` (unknown) together, and `relay-watcher` refuses at the first
bad key's line. The four new doctor codes are pinned beside the OTLP ones with no collision. The
heartbeat format is unchanged by this PR (`health.rs` is not in the diff). Default off holds: the
built `relay-watcher --once` with a fake `HOME` and `XDG_RUNTIME_DIR` and no `delivery` key made
no access to `.claude`, `cc-socks` or `state/socket` under strace and created nothing; with
`delivery = "socket"` it created only `state/socket` (0700) and a 0600 status file, removed at
exit. The production diff adds no `unwrap`/`expect`, `allow` or `unsafe`, and nothing under
`.github`.

The two items observed in pass 2, re-checked. (a) The `settlement_failed` note is now keyed by
lead and batch (`failed_notes: BTreeMap<(String, u64), String>`) and cleared only by that
batch's successful write; `tc_044_11_a_settlement_failed_note_is_cleared_only_by_its_own_batch`
passes. Not persisted across a restart, which FR-044 now states ("the note is not saved in the
status file, so after a restart it is raised again after five further failures"): resolved.
(b) A refused status directory now logs and alerts once per episode (`status_failing`), delivery
goes on, and FR-044 states it; `tc_044_15_an_untrusted_status_directory_is_alerted_once_and_delivery_goes_on`
passes: resolved.

## Dispositions

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed | e002abf: `transcript::find(..).map(Watch::new)` now runs before `write_all` (deliverer.rs:777-784). Probe with a blocking receiver that appends the line the instant it reads, pre-existing transcript, 0/20/200 extra project dirs, 200 rounds each: 0 misses. Mutant (watch moved after the write) fails both the probe (199/200) and `tc_044_04_a_receiver_that_records_at_once_into_an_existing_transcript_is_never_missed`. |
| FND-002 | fixed | e002abf: budget cuts are `SocketError::Postponed` / `DeliverError::Postponed` / `Outcome::SocketPostponed` with no episode and no notification (watch.rs:181-186, 361-365); probe deferred-then-held gives one notification. The fix introduced FND-009. |
| FND-003 | fixed | e002abf: a failing decision is never given up; it is renewed while owed (`held_since.is_some() \|\| (resolution.is_some() && attempts > 0)`), retried after 5 s doubling to 5 min, one alert and `settlement_failed` note at the 5th failure (deliverer.rs:1059-1101, 1237-1257). Connection errors are counted (`applied` runs before the match in main.rs `apply_settlements`). `tc_044_11_a_dead_letter_write_that_fails_for_ever_never_re_posts_the_message` passes on Redis: 20 ticks each past a lease, reaped 0, posts 1, alerts 1. |
| FND-004 | fixed | e002abf: `HeldBatch.delivered_since_ms` is persisted and adopted into `delivered_at` (socket_status.rs:118-122, deliverer.rs:519-535). Full-flow probe: held, late `delivered`, deliverer dropped, file re-labelled as a dead watcher's, restart with max hold 0.4 s and grace 1.5 s: 0 dead letters, acked `delivered_unconfirmed` at 1.48 s. |
| FND-005 | fixed | e002abf: the whole attempt (resolve, connect, find, write, wait) is counted, the pre-post check is `tick_spent + elapsed + post_worst_case() > tick_budget` with `TICK_BUDGET` 8 s and a const assert on connect+write+wait, and the doc states the uncounted transcript scan. The per-call write timeout remains (FND-011). |
| FND-006 | fixed | e002abf: leads are rotated by `state.rotation % local.len()` each tick when the transport rotates (main.rs:842-850). `tc_044_14_a_healthy_lead_behind_twenty_wedged_ones_is_served_within_one_round_of_ticks` passes at production constants with suspicion lapsed every tick: served within 21 ticks. |
| FND-007 | fixed | e002abf: `checked_sub` in `prune_dead_notes` and adoption, `ms_ago` saturating on write (deliverer.rs:417-429, 486-525, 1440-1442); `tc_044_13_times_at_the_edge_of_i64_in_a_status_file_are_dropped_not_computed` passes (debug build, no panic). |
| FND-008 | fixed | e002abf: status files moved to `state/socket` (`dir_in`), an existing dir is `set_permissions` 0700 before writing, and a group/world-writable dir is not read (socket_status.rs:27-39, 198-208); tested on a pre-existing 0755 dir. Trust-check gaps are FND-010. |
| FND-009 | still-open | New this round; not addressed. A postponement ends a `SocketHeld` episode and causes a second bypass notification. |
| FND-010 | still-open | New this round; not addressed. Trust check ignores owner and symlinks; the watcher's chmod blesses files planted while the dir was open. |
| FND-011 | still-open | New this round; not addressed. The write bound is per call; one post measured 14.06 s against the stated 7 s worst case. |
| FND-009 | fixed | dc4ccd5: `observe_outcome` returns an empty transition for `Outcome::SocketPostponed`, and `episode_lines` logs nothing for it. Full-flow probe (watch.rs fakes, `deliver` + `observe_outcome` + `episode_lines`), held / postponed / held / postponed / held: 1 notification, 0 "ended" lines. `tc_044_14_a_postponement_leaves_a_held_episode_untouched` is mutation-checked. |
| FND-010 | fixed | dc4ccd5: the directory is trusted only when `symlink_metadata` shows a real directory owned by this uid with mode & 0o077 == 0. It is created with `DirBuilder::mode(0o700)` only when absent, never chmodded, and refused otherwise. A status file is read only when it is a regular file of this uid, and the temp file is `create_new`. Probes: a symlinked `state` dir pointing at a 0700 dir with a forged held batch and owed requeue adopts nothing and the forged file is left, while socket delivery still works. A symlinked status file in a 0700 dir is not adopted. `tc_044_15_a_file_planted_in_an_open_directory_is_never_adopted_even_after_a_start` passes (0777 dir, two starts, mode unchanged). |
| FND-011 | fixed | dc4ccd5: `write_within` makes the stream non-blocking and keeps one 2 s deadline for the whole line. A timeout marks the lead suspect and returns `Ambiguous`, and the stream is dropped. Probe, 900 KiB to a receiver reading 128 KiB every 1.8 s: gave up after 2.10 s. The receiver got 365,440 bytes, then a clean EOF with no newline, so a partial line is never complete. The next post to the lead is `NotSent` (herdr, 10 min). No pending entry is made, so the batch is redelivered marked only when its lease lapses. |
| FND-012 | still-open | New this round; not addressed. Owed ack/requeue from a status file bypass the popper guard. |
| FND-012 | fixed | 1df698c (the squashed commit; pre-squash fix 12df7e7): `Relay::ack` and `Relay::requeue_batch` take a `Popper`, `ack.lua` and `requeue.lua` match `who == ARGV[...]`, `apply_settlements` and the live `RelayInbox` paths pass `Popper::Watcher` for all four batch scripts, and the hook passes `Popper::Hook`. Replayed on Redis: the hook holds batch 1; an owed `ack`, `requeue` or `dead_letter` from a dead watcher's file leaves it in flight (1, 0, 0) with nothing re-delivered. Mutants (either script without the popper match) fail `tc_044_13_an_owed_ack_or_requeue_never_touches_the_hooks_batch`, so the owed entry is adopted, not vacuous. Mixed versions: an older binary uses its own id-only script on its own batch ids; the owed path exists only in this build. |
