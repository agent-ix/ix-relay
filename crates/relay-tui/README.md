<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- Copyright (C) 2026 Agent-IX -->

# Team Leader Dashboard

Monitor one measured native lead session:

```sh
cargo run -p relay-tui --bin team-leader-dashboard -- --lead luna:SWM:code --session <native-session-id>
```

Scope defaults to the bound workspace team (for example IR); `--team IR` overrides
the team when a workspace label differs. Explicit initiative, project or ticket
selectors narrow that team boundary. Unprojected team tickets count in whole-team
scope. Project focus lists its project, with associated initiatives in details.

Focus and agent assignments are explicit lead reports written with
`agent-msg work`. Open tickets alone do not prove agent activity. The view shows
scoped leaf-task progress, project counts, ready work and agent assignments.
Initiative counts are restricted to the same scope. Internal agent activity is
reported; pane activity is observed only when the native session matches.

## Refresh intervals

- Linear: immediately at startup, then every **30 minutes** through ix-board's
  structured cache API. Rate-limited reads retry on that cadence with a live
  countdown. The full rolling-quota reset does not suspend reads for an hour.
- Herdr activity: immediately at startup, then every **minute**.
- Redis reports: immediately at startup, then every **minute**, independently
  of Herdr. These reads check scope, focus and agent assignments, not Linear.

`r` requests an immediate Linear fetch unless rate limited, in which case it
coalesces with the next scheduled attempt. `s` toggles source timestamps (hidden by default). Errors are always visible.
`d` or Enter toggles details;
arrows, j/k and Page Up/Down select assignments. `q` or Escape quits.

Tracker failures preserve visibly labeled last successful snapshots and their
original source age. A snapshot is labeled stale at the 30-minute polling
interval or after a read failure. Herdr activity becomes unknown after two
minutes without a successful observation. A replaced native session requires
an explicit restart to bind its successor. The monitor does not change Linear
or register agents on the message bus.

To restore whole-team scope, report `agent-msg work scope --all-team`. This clears
narrower selectors and preserves focus and agent assignments.
