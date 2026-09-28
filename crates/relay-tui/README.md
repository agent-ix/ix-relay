<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- Copyright (C) 2026 Agent-IX -->

# Team Leader Dashboard

Run the read-only team leader dashboard from an ix-relay checkout. The current
deployment is configured for team SWM and its ix-board and ix-projects projects.
Set `LINEAR_API_KEY` for tracker data:

```sh
cargo run -p relay-tui --bin team-leader-dashboard
```

The view focuses on the current lead label, assigned projects and ticket work.
For ix-board and ix-projects it shows project status and active/completed issue
counts, current open SWM tickets, their direct blockers, and the next actionable
ticket in ix-board's deterministic ready queue. `r` refreshes tracker data; `q`
or Escape exits. `AGENT_MSG_NAME` supplies the lead label when set; otherwise the
view uses `SWM team lead`.

The dashboard reads Linear through ix-board's structured Rust API pinned at
`29a12f5951f1634dabd654495ed44102eee312f5`. It uses ix-board's cached workspace
snapshot and reuses its cache path; ix-board refreshes that snapshot after its
five-minute cache expires. After a failed tracker read, retries back off from
30 seconds up to 15 minutes; `r` requests an immediate read. Project status,
ticket state, priority, and `blocked_by` relationships come from the structured
snapshot. The dashboard is read-only and does not parse ticket prose or call a
Linear mutation API.

The lane signal is derived only from the assigned projects' structured ticket
and direct blocker state. It reports ready work when ix-board supplies an
actionable next ticket, reports blocked when displayed open work has unresolved
direct blockers, and reports unknown when the tracker is stale, unavailable, or
missing either assigned project. Staleness is reported after 90 seconds without
a successful dashboard read; the underlying ix-board snapshot can be up to five
minutes old because of its cache. Tracker errors and missing or stale state are
shown explicitly rather than presented as progress.

The dashboard does not poll or display Herdr sessions, agent activity, relay
registrations, or bus messages. Its palette follows the Agent-IX Cloud Command
tokens (blue primary, orange accent, green ready, yellow blocked) and uses a
compact diamond glyph in place of a graphical icon. Filament IDE's icon and
toolbar examples are visual references only; no assets are copied into this
repository.
