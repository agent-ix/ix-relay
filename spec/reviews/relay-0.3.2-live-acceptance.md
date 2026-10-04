# ix-relay 0.3.2 live acceptance

Date: 2026-10-04. Candidate executable revision:
`e101d3a31a61d20d5084589837e850b032980534`. Release tagging is pending.

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

- Draco PID 96474, SHA-256 `be3a5d7a3f1d179015435649191a47b186b709e6cf454a24cbd01ea8e4fa0fc3`.
- Luna PID 93145, SHA-256 `e5d519cdb6beb08bf2fbd186b124e08815dc3650fce185d3fc5cfd3ce1d08ac0`.
- Terra PID 1697168, SHA-256 `2574a999a8bac905bce8b4506856c8fdceb83fd78cb2135cdeb84c65cd6e5f42`.

These are candidate rollout facts, not a claim that release acceptance is
complete. Pending gates include normal Codex-hook receipt, final test evidence,
tag publication and final installed-revision verification. OpenCode's known
adapter limitations are documented in relay-directory-state-model.md.
