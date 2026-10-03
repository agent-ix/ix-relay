# ix-relay Herdr plugin

The plugin exposes the same ix-relay status command from any Herdr pane. It
does not install agent-specific hooks or start an agent turn.

## Install

Build and install `agent-msg` and `relay-directory` so both commands are on the
Herdr server's `PATH`. Then link this directory:

```bash
herdr plugin link /absolute/path/to/ix-relay/plugins/herdr
```

Herdr's plugin link is per user and survives Herdr restarts. To show the
indicator at the right of the tab row, add this to `~/.config/herdr/config.toml`:

```toml
[ui]
tab_bar_right = [
  { type = "command", command = "agent-msg badge", interval_seconds = 5, timeout_seconds = 2 },
]
```

Then reload the Herdr configuration or restart Herdr. The badge command
prints relay state and the verified focused lead's pending count; if identity
or Redis cannot be verified, it prints `pending unknown`. `agent-msg status`
prints one compact line; `agent-msg status --json` prints the structured
snapshot for other integrations. Herdr removes terminal control sequences
from command output, so the indicator uses text and symbols rather than ANSI
color codes.

For the optional `$ix_relay` sidebar row, see [the repository README](../../README.md#optional-herdr-sidebar-badge).

The snapshot distinguishes Redis reachability, the local watcher heartbeat,
and whether the focused pane's live Herdr session matches a relay registration.
It reads no inbox and does not register a lead. Run `agent-msg doctor` for the
full diagnostic report.

## Browse online leads

Open the searchable popup from any lead pane:

```bash
herdr plugin action invoke agent-ix.ix-relay.open-directory
```

Type to filter by lead, alias, machine, workspace, tab, pane, session, or
address. The list shows Herdr activity, exact relay registration state, last
registration refresh, and each machine's watcher freshness. When Herdr cannot
read a configured machine, the popup marks results incomplete and leaves that
machine's leads unknown. Select a row and press `y` to copy its canonical
address when the live Herdr session matches a relay registration. Press `F5`
to refresh or `q` to close. The popup refresh reads only Herdr and relay
metadata; it does not read or modify any inbox. `agent-msg inbox --peek`
provides a bounded non-destructive preview for other integrations.

Press `m` from the directory to open recent formatted history and a pending
preview. The popup verifies the focused lead first; pending messages are read
without being removed. Press `r` to compose a reply to the oldest pending
message. Before sending, the popup verifies the focused lead again and resolves
the sender to exactly one live canonical address. Message bodies are displayed
as plain data.
