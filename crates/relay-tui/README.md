<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- Copyright (C) 2026 Agent-IX -->

# SWM work tracker monitor

Build and run the read-only monitor in a Herdr pane from an ix-relay checkout.
Set `LINEAR_API_KEY` for tracker data and configure the usual ix-relay Redis
settings for relay context:

```sh
export LINEAR_API_KEY=…
cargo run -p relay-tui --bin herdr-monitor
```

The main view foregrounds lead `luna:SWM:code`, team SWM, and the ix-board and
ix-projects project status, active/done counts, current open tickets, and the
next ticket from ix-board's ordered ready queue. Active counts include every
issue except Completed and Canceled; done counts include Completed. Current open
tickets include all nonterminal SWM issues in the selected projects except the
selected next ticket, and are shown in Linear identifier order. The next ticket
is the first item in ix-board's ordered queue for either selected project that
is not in Backlog and whose direct blockers are all Completed. This display
guard excludes deferred Backlog work and rejects queue entries that contradict
their structured blocker state. Direct blockers are reported as blocked until
Completed; missing references and stale or unavailable data are shown as
unknown. The lane signal reports `NEXT TICKET READY` only when such a candidate
exists; this does not assess schedule or overall project health. The monitor
does not calculate ix-board's historical drift report, which requires
additional evidence.

The monitor reads Linear through ix-board's structured Rust API pinned at
`30dd492f3676cdfe43c8f6a7824ef3a652e8cc61`. It uses ix-board's cached workspace
snapshot and reuses its cache path; ix-board refreshes that snapshot after its
five-minute cache expires. This avoids repeatedly forcing full-workspace reads
and consuming Linear's request allowance. After a failed tracker read, the
worker doubles its retry interval from 30 seconds up to 15 minutes; `r` still
requests an immediate read. Project
status, ticket state, priority, and `blocked_by` relationships come directly
from that structured snapshot; the monitor does not parse ticket prose or call
any Linear mutation API. `r` requests a refresh and `q` or Escape exits.

Herdr's structured `herdr api snapshot`, configured ix-relay Redis bus
registrations, and recent messages appear below as supporting context. The
current Herdr pane line uses `HERDR_PANE_ID`; if the ID is missing from the
snapshot, that context says unavailable. Collaboration subagents appear only
when they register in Herdr or relay.

All source reads run on the background worker so terminal input stays
responsive. Herdr and Redis data older than 15 seconds are marked stale. Linear
read status becomes stale after 90 seconds without a successful read; the
underlying ix-board snapshot can be up to five minutes old because of its cache.
A failed or stale Linear read makes the project counts, ticket panels, and lane
signal show unknown instead of implying progress. Terminal text is sanitized
before it is rendered.

The terminal palette follows the Agent-IX Cloud Command tokens (blue primary,
orange accent, green ready, yellow blocked) and uses a compact diamond glyph in
place of a graphical icon. The visual references are
`filament-ide-rs/ui/src/cockpit.css`, its inline Lucide-style toolbar icons, and
`crates/filament-tray/assets/menubar-mask.gray`; no assets are copied into this
repository. The named repository is not available as a local checkout on the
build host.
