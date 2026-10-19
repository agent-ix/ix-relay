---
id: NFR-001
title: "Hook time budget per event"
type: NFR
quality_attribute: performance_efficiency
relationships:
  - target: "ix://agent-ix/ix-relay/FR-017"
    type: "constrains"
  - target: "ix://agent-ix/ix-relay/FR-018"
    type: "constrains"
---
# NFR-001: Hook time budget per event

## Statement

The hook handler shall finish every call whose payload is complete within 300 ms
of the start of the call within its event's total budget, counted from the start
of the call — 300 ms for `UserPromptSubmit`, 900 ms for `PostToolUse` and `Stop`,
and 3300 ms for `SessionStart` — whether Redis and herdr answer, hang or refuse.

The hook handler shall pop the inbox only with at least the window its reply needs
(twice the round trip it measured plus 20 ms, or 200 ms when it measured none: the
pop's reply and the acknowledgement that follows the write).

While the hook has no complete hook payload it does not know its event, so the
handler shall keep reading for as long as bytes arrive at least every 300 ms, and
shall end the call at the first 300 ms without new bytes, or at 900 ms from the
start of the call, whichever comes first, with one recorded `decode`/`stdin_timeout`
failure, nothing registered or popped and exit 0. The 900 ms is the largest budget
of an event the harness kills at 2 s, so no call that reads stdin outlasts it.

If a payload completes after its event's budget is spent (a `UserPromptSubmit`
payload after 300 ms), then the handler shall register and pop nothing, make no
connection, record one `budget`/`budget_exhausted` failure and exit 0. That call
takes as long as the payload does, at most 900 ms, not its event's 300 ms.

## Scope

- Applies to `agent-msg hook claude` and `agent-msg hook codex`, for every lead
  and every Redis and herdr condition.
- The budget covers reading hook input, the Redis connection, identity
  derivation (herdr calls and the transcript scan), registration, the pop and the
  address-book read.
- Reading hook input ends at the first of: the end of the first JSON object (the
  harness need not close the pipe), end of file, 8 MiB, 300 ms without new
  bytes (the window restarts whenever bytes arrive, so a large payload written
  slowly is not a timeout while it progresses), or 900 ms from the start of the
  call. Once the payload is in, the event's own budget applies, counted from the
  same start, so a payload whose last bytes arrive at 250 ms leaves a `UserPromptSubmit` 50 ms,
  and one that completes at 500 ms leaves it nothing (a recorded `budget_exhausted`).
  The cap is 8 MiB, the bound this repo uses for untrusted local reads
  (`MAX_TRANSCRIPT_SCAN_BYTES`), because a `PostToolUse` payload carries the tool's
  input and response (an Edit or Write of a large file, a Bash with long output),
  which is megabytes, not the few hundred bytes of the other events; more than
  that is a recorded `decode`/`too_large`. The end of the object is found by a
  one-pass scan (brace depth, string and escape state), so reading is linear in
  the bytes read: 8 MiB reads and decodes in about 0.1 s in a debug build and
  0.02 s in a release build, well inside the 300 ms window.
- Bytes after the first object (a second value, garbage, a NUL) are ignored.
  Invalid UTF-8 inside a string, input that is not an object, and an object that
  is not a hook payload (`{}`, no `hook_event_name`) are `decode`/`malformed`,
  immediately, also on a pipe that stays open.
- After a decode failure that repeats on every call the hook makes one attempt to
  ship the spool, within what is left of 300 ms counted from the start of the call.

## Rationale

`PostToolUse` and `Stop` exist to deliver mail, so their budget must not be what
stops delivery: 900 ms leaves room for a cache hit (`3 x rtt + 20 ms`) up to about a
290 ms round trip and for a cache miss to ask herdr first. The price is that a dead
SSH tunnel costs a tool call 900 ms. `UserPromptSubmit` never calls herdr and its
mail also arrives at the next `PostToolUse` or `Stop`, so it keeps 300 ms and
defers its pop silently over a slow link. A payload that is still arriving is read for
up to 900 ms whatever the event, because the event is not known until it is in; so
the 300 ms of `UserPromptSubmit` binds a call whose payload is in within 300 ms, and a
late one ends the call by 900 ms at the latest, still inside the harness's 2 s. The plugin manifests give the harness a
hard timeout of 2 s (5 s for `SessionStart`), and a hook killed by the harness
leaves its popped batch in flight until the lease returns it (a duplicate, not a
loss), so the hook's own budget sits well inside it.

## Supported round trip

Each pop is issued only when its window fits, so a slower link defers or records a
skipped pop and never loses mail. The sender-address read after the pop is issued
only when a further window is left, and otherwise bare bus names are printed: the
address read gives way before the acknowledgement after the write does. Delivering links,
by event: `UserPromptSubmit` about 56 ms for a derived name (`5 x rtt + 20 ms <= 300
ms`, with sender addresses up to about 46 ms) and about 70 ms for an explicit one
(addresses up to about 56 ms); `PostToolUse` and `Stop` about 290 ms for a cache hit; `SessionStart`
far beyond any tunnel. A path that made no request of its own before the pop
(an explicit name at `PostToolUse`/`Stop`) uses a 200 ms window, so it delivers to
about 90 ms and beyond that skips and records the pop.

## Measurement and Evaluation

| Metric | Target | Threshold | Method |
|--------|--------|-----------|--------|
| `UserPromptSubmit` wall time, Redis blackholed | 300 ms | 400 ms | performance-benchmarking |
| `PostToolUse` and `Stop` wall time, Redis blackholed | 900 ms | 1000 ms | performance-benchmarking |
| `SessionStart` wall time, Redis and herdr blackholed | 3300 ms | 3400 ms | performance-benchmarking |
| `UserPromptSubmit` wall time with a 200 MB transcript and no `/rename` line | 300 ms | 400 ms | performance-benchmarking |
| Wall time with stdin held open and no complete payload (silent), ending in one `stdin_timeout` record | 300 ms | 400 ms | performance-benchmarking |
| `PostToolUse` and `Stop` wall time on a session-cache miss, Redis answering and herdr hanging | 900 ms | 1000 ms | performance-benchmarking |
| Wall time with a byte arriving every 200 ms and no complete payload (drip), ending in one `stdin_timeout` record | 900 ms | 1000 ms | performance-benchmarking |

The threshold allows 100 ms for process start-up and exit on top of the budget.

## Verification

A benchmark harness runs the hook binary against a Redis address that accepts
connections but never answers, a herdr stub that never exits, a large transcript
file and a stdin that is never closed or never written, and asserts every run's wall time is within
threshold and every run exits 0.

## Dependencies

- **Upstream**: [FR-017](../functional/FR-017-hook-never-fails-the-turn.md),
  [FR-018](../functional/FR-018-hook-delivers-per-event.md)
- Delivered in part by AGE-2018.
- The transcript scan reads at most the last 8 MiB of the transcript
  (AGE-2026), which bounds the fourth row by construction; it is still to be
  measured.
- Stdin is read on a helper thread bounded by the deadline (AGE-2033). Why a
  limit at all: the harness kills `PostToolUse`, `Stop` and `UserPromptSubmit` at 2 s
  (`plugins/claude/hooks/hooks.json`), so a longer wait could not leave a record.
  Why 300 ms without bytes: both harnesses write the payload at once and close
  stdin (the former `read_to_string` worked live, and
  the retired pre-0.3 live driver read to end of file), so a payload that has
  not started, or has stalled, for 300 ms is not coming. Why the window restarts
  and a cap of 900 ms (AGE-2063): a total 300 ms limit recorded a large payload
  written slowly on a busy host as a timeout and delayed that turn's mail, although
  it was arriving; and 900 ms is the largest event budget the harness kills at 2 s.
  The cost is that a `UserPromptSubmit` payload that completes after 300 ms is read
  and then recorded as `budget_exhausted` (its mail comes at the next `PostToolUse`
  or `Stop`), and a `SessionStart` payload that is still incomplete at 900 ms is
  recorded (the next `UserPromptSubmit` registers). AGE-2028 kept the budgets and sized the pop window from the link (see "Supported round trip").
