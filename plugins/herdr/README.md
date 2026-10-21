# ix-relay Herdr plugin

The plugin exposes the same ix-relay status command from any Herdr pane. It
does not install agent-specific hooks or start an agent turn.

## Install

Build and install `agent-msg` first so `agent-msg` is on the Herdr server's
`PATH`. Then link this directory:

```bash
herdr plugin link /absolute/path/to/ix-relay/plugins/herdr
```

Herdr's plugin link is per user and survives Herdr restarts. To show the
indicator at the right of the tab row, add this to `~/.config/herdr/config.toml`:

```toml
[ui]
tab_bar_right = [
  { type = "command", command = "agent-msg status", interval_seconds = 5, timeout_seconds = 2 },
]
```

Then reload the Herdr configuration or restart Herdr. The status command
prints one compact line; `agent-msg status --json` prints the structured
snapshot for other integrations. Herdr removes terminal control sequences
from command output, so the indicator uses text and symbols rather than ANSI
color codes.

The snapshot distinguishes Redis reachability, the local watcher heartbeat,
and whether the focused pane's live Herdr session matches a relay registration.
It reads no inbox and does not register a lead. Run `agent-msg doctor` for the
full diagnostic report.
