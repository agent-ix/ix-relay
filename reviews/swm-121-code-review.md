---
id: SR-6127
title: "Code review of ix-relay PR 30: socket-only delivery to Claude Code sessions (SWM-121 to SWM-124)"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; PR #30 at f2cbcc8; crates/relay-watcher/src/**, crates/agent-msg/src/{claude_inbox,hook_run,main}.rs, crates/agent-msg/src/doctor/**, crates/claude-wire/src/{record,transcript}.rs, crates/relay-core/src/{config,receipts,socket_status}.rs, crates/relay-core/src/store/lua/renew.lua"
review_set: subset
---

# SR-6127: Code review of socket-only delivery (SWM-121 to SWM-124)

## Summary

Ticket: SWM-121 (with SWM-122, SWM-123, SWM-124). PR agent-ix/ix-relay#30 at `f2cbcc8e48953c5d7d5c41b515534c1642b8773b`, reviewed as `git diff origin/main...HEAD` against `ea441b6` (40 files, +1935/-627). The rust-review lane is folded into this file.

Typing paths: the only pane-input call is `herdr agent prompt` in `relay-watcher/src/herdr.rs`, reached only on `Route::Terminal`, and `Routed::deliver_to` asks the route again immediately before it. No path was found that types into a Claude Code session with a socket in default mode when its registration kind is `claude`. The two exceptions are the operator override `delivery = "herdr"` (FND-006) and a registration whose kind is not exactly `claude` (FND-005). Parked, young, reused-pid, stale, missing, unreadable, ambiguous and unsupported records all wait. A config that does not parse stops the watcher and leaves hooks in socket mode, so it does not fall back to typing.

Root cause: the live registry on draco still shows session `93c972c8` as an interactive record with `parkedJobId`, consistent with the stated mechanism; which conversation the pane showed on 2026-11-24 was not verified. `tc_044_20` was traced against main: there the parked record resolves, the silent socket marks the lead suspect, and the second batch is typed through Herdr, which is the failure the PR quotes. It was not executed on main (one worktree, read only).

Compatibility: old status files load (`awaiting_consumption` defaults to false); old hooks with a new watcher cannot duplicate (pops are atomic). New hooks with a not yet restarted old watcher leave working leads' mail to be typed when idle, which the PR's install note covers. Non-Claude harnesses keep the Herdr readiness checks. Against main, the only removed functions are nine tests of the removed Herdr fallback; fleet, relay-ops, activity and receipt scripts are unchanged.

Gates, run in the one worktree against an isolated Redis 8.10.2 with isolated HOME and XDG_RUNTIME_DIR: fmt-check, clippy -D warnings, deny licenses, audit-unsafe, check-matrix, docs and `cargo test --workspace` all passed (1666 passed, 0 failed, 21 ignored). `make test-redis` (Docker redis:7) was not run.

## Verdict

**CONDITIONAL**: no high findings. Four medium findings, three of them ways a lead can stop receiving mail that it received before this change, and three low ones.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | A Claude lead the watcher cannot serve receives nothing, indefinitely and with no push signal. The hook stops delivering whenever `CLAUDE_CODE_MESSAGING_SOCKET` is set (always, for a session with a socket) or any record names a socket, while the watcher routes the same session to Wait for a record under another `CLAUDE_CONFIG_DIR`, an unsupported version or `peerProtocol`, several live processes, a parked session or a refusing receiver (re-posted forever). Mail stays queued; the only signals are one log line per stretch and a doctor warning. No notification is raised, unlike a blocked or held lead. A Claude Code update that changes `peerProtocol` would stall every Claude lead this way; on main their hooks and Herdr delivered. A parked lead waits even though Claude Code itself follows `parkedJobId` to the job's socket. Raise one `herdr notification show` (not typing) per episode when mail has waited past a bound. | crates/agent-msg/src/claude_inbox.rs:48-61; crates/agent-msg/src/hook_run.rs:848-851; crates/relay-watcher/src/socket/deliverer.rs:941-942; crates/relay-watcher/src/watch.rs:347-348 |
| FND-002 | medium | A failing Herdr shape check blocks socket delivery too. `tick` skips every lead while `delivery_blocked()` is true, before the route is asked, although the socket route needs no Herdr read and FR-036 only forbids typing into a pane. With hooks no longer delivering for socket sessions, a Herdr upgrade that breaks decoding stops all mail to idle and working Claude leads while their sockets are usable. Ask the route first and apply the block to the terminal route only. | crates/relay-watcher/src/main.rs:1131-1135 |
| FND-003 | medium | A posted batch whose receiving process is gone is never re-sent. `settle` renews a pending post until 30 minutes and then dead-letters it as `consumption_expired` without checking that the process it was posted to still exists. A lead that exits, crashes or restarts while the message is queued in the old process loses it to the dead-letter list, also when the write itself failed on a closed socket and nothing was sent. On main the lease expired and the batch was redelivered. A lead that sits on a dialog or in one long tool call for over 30 minutes is dead-lettered as well although Claude Code may consume the copy later. When the pid and start time are gone and the transcript does not show the id, requeue instead. | crates/relay-watcher/src/socket/deliverer.rs:1137-1146; crates/relay-watcher/src/socket/deliverer.rs:1640-1675 |
| FND-004 | low | Restart can misjudge a pending post. An adopted entry is watched only from the last 512 KiB of the transcript, so a message consumed during the restart with more than that written after it is dead-lettered as `consumption_expired` although consumed. An adopted entry older than the maximum hold is dropped from adoption, so its lease lapses and the batch is posted again, which FR-044 says never happens. Only held batches were persisted before this change; now every unconsumed post is. | crates/relay-watcher/src/socket/deliverer.rs:631-640; crates/relay-watcher/src/socket/deliverer.rs:1554-1562 |
| FND-005 | low | A registration whose kind is not exactly `claude` is typed into even when its session has a socket. The kind comes from Herdr's `agent` member and becomes `unknown` when Herdr omits it, while the Claude hook (which knows it is Claude) leaves the mail to the watcher, so every message would be typed. FR-054 specifies this; the owner's requirement does not allow it. Waiting when a live Claude record names the session would close it. Not observed with Herdr 0.9.1. | crates/relay-watcher/src/socket.rs:135-140; crates/relay-ops/src/find.rs:631 |
| FND-006 | low | `delivery = "herdr"` still types into every lead and doctor reports it as `ok`. Main's sample config in docs/configuration.md carried the line uncommented, so a machine set up from it keeps typing after the upgrade with nothing flagging it. Report it as `warn`, and check each machine's config at install. | crates/agent-msg/src/doctor/checks.rs:161-167 |
| FND-007 | low | Doc comments and the README contradict the new behavior: `SocketStatus.reply_socket` says every batch then goes to Herdr, `notes` says these batches go to Herdr, `DeadLetterNote.reason` omits `consumption_expired`, and the README opening says the watcher types queued messages into idle leads. | crates/relay-core/src/socket_status.rs:180; crates/relay-core/src/socket_status.rs:528-529; crates/relay-core/src/socket_status.rs:550-551; README.md:5 |
