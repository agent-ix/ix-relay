# ix-relay 0.3.2 live acceptance

Date: 2026-10-04. Candidate executable revision:
`e101d3a31a61d20d5084589837e850b032980534`. Release tag: `v0.3.2` (also `ix-relay--v0.3.2`).
Executable code corresponds to the candidate revision above; subsequent
commits add acceptance evidence only.

## Verified cross-server evidence

The real Luna IR planner ran agent-msg 0.3.2 and reported verified identity
`luna:IR:plan`. `herdr machine list` remained `No saved SSH machines`.
No remote profiles were added and the planner sent without `find` first.

At 11:11 PDT, the planner sent `IXR032-LUNA-TERRA-20261004` to
`terra:QSL:plan`, exit 0, queued once. Terra's planner explicitly acknowledged
normal receipt to Luna and Draco. Luna subsequently confirmed receiving that
acknowledgement. This proves the Luna-to-Terra round trip through Redis.

At 11:11 PDT, the planner sent `IXR032-LUNA-DRACO-20261004` to
`draco:ix-relay:code`, exit 0, queued once. That proves enqueue, not yet normal
Codex-hook receipt. This Codex lead still has pending messages; its receiving
integration remains under investigation. At 11:14 PDT, Luna sent `IXR032-LUNA-DRACO-CLAUDE-20261004` to
`draco:misc:mods`. That Claude lead explicitly acknowledged normal socket
receipt to Luna and Draco. Luna confirmed receiving the acknowledgement.
This proves the Luna-to-Draco round trip independently of the Codex inbox.

## Genuine Codex startup and receipt

An owned temporary tab `draco:ix-relay:receipt-test-032` ran Codex 0.160.0
with explicit `--no-daemon`. Genuine SessionStart associated session
`01a10824-a697-77f0-8cd8-e24c765075b0` with pane `wT:p5`. No manual
registration or inbox reads were performed. A cold send succeeded.

- `IXR032-CODEX-RECEIPT-20261004`: normal idle delivery appeared in the
  actual terminal and the Codex lead acknowledged it through agent-msg.
- `IXR032-CODEX-ACTIVE-20261004`: the lead received normal hook context
  after a 12-second shell tool and acknowledged it.
- `IXR032-CODEX-GOAL-20261004`: the lead received normal hook context
  during an actual temporary goal and acknowledged it.

The terminal output and relay log agree. The owned fixture tab was closed
after completion. These checks prove the fresh installed Codex path works;
they do not explain this older lead's still-pending inbox. A read-only
`hooks/list` request using the same installed binary reported the configured
relay definitions enabled/trusted with no warnings or errors. Both genuine
session-cache entries exist. Shared hook-health records show this older lead's
real PostToolUse hooks have run, with recorded Redis connect timeouts. The
absence of rollout hook events is therefore not proof they were never invoked.
The operator suggested a restart and explicitly accepted fresh-session tests
as sufficient to proceed with Luna rollout. Its existing inbox is preserved;
this release does not claim those queued messages were normally received.

## Production regression found by the live test

The first request to Luna failed before enqueue because the production
`Routed<RealHerdr>` delivery adapter did not forward directory observation
authority. Local cold recovery had masked missing watcher publications.
The correction forwards authority for both socket and Herdr delivery modes.
A regression runs the actual watcher executable with both configurations
against isolated Redis and verifies a leased publication. Targeted tests pass
on macOS and Linux. The complete corrected watcher suite passed all 317
tests (303 unit and 14 process/startup tests) on macOS, and all 315
tests (301 unit and 14 process/startup tests) on Luna/Linux.

## Installed candidate

Corrected native watchers are active on all three machines:

- Draco supervised LaunchAgent PID 13262, SHA-256 `be3a5d7a3f1d179015435649191a47b186b709e6cf454a24cbd01ea8e4fa0fc3`.
- Luna PID 93145, SHA-256 `e5d519cdb6beb08bf2fbd186b124e08815dc3650fce185d3fc5cfd3ce1d08ac0`.
- Terra PID 1697168, SHA-256 `2574a999a8bac905bce8b4506856c8fdceb83fd78cb2135cdeb84c65cd6e5f42`.

All native binaries and on-disk Claude/Codex integrations report 0.3.2 on
Draco, Terra and Luna. Terra and Luna services are active; Draco is supervised
by its enabled LaunchAgent. Running agents may retain their previously loaded
plugin instructions until their next authorized fresh session.

At release time, the older Draco Codex lead still had pending messages.
The post-release normal-hook receipt below clears that acceptance concern;
a restart is no longer required solely on the basis of the pending inbox. OpenCode's known adapter
limitations are documented in relay-directory-state-model.md.

## Final smoke then IR check

Luna smoke after the corrected rollout: native agent-msg and watcher 0.3.2,
Redis answers, all three watcher heartbeats report 0.3.2, Luna service active,
and `No saved SSH machines`. Historical hook-health warnings remain preserved;
this is not a claim that doctor's complete historical report is green.
The post-smoke IR check uses `IXR032-LUNA-FINAL-ROUNDTRIP-20261004`.

At 11:29 PDT, Luna sent the final nonce once to Terra QSL plan and Draco
misc mods. Both leads explicitly acknowledged normal receipt to Luna. Luna
confirmed receiving both acknowledgements, first-attempt enqueue exit 0, no
repeats, no `find` and no SSH changes. Final post-smoke IR acceptance passed.

## Post-release receipt in the original Codex lead

The original `draco:ix-relay:code` lead subsequently received the complete
acceptance batch through its normal receiving hook, including
`IXR032-SELF-20261004`, `IXR032-LUNA-DRACO-20261004`, the Terra and Draco
Claude acknowledgements, all three fresh Codex fixture acknowledgements,
and Luna's final post-smoke round-trip report. No manual inbox pop or
fabricated hook event was used. `whoami --json` still reports the verified
`draco:ix-relay:code` address. An acknowledgement was queued to `luna:IR:plan`.

This proves normal receipt in the original lead as well as the fresh fixture.
Earlier pending-inbox and timeout observations are retained as historical
facts, not a diagnosis that the current session needs a restart. This evidence
addition does not move the published 0.3.2 tags or change executable code.
