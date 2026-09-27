<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- Copyright (C) 2026 Agent-IX -->

# Herdr activity monitor

Build and run the read-only monitor in a Herdr pane from an ix-relay checkout:

```sh
cargo run -p relay-tui --bin herdr-monitor
```

Open a new Herdr pane in the workspace/tab you want to observe, then run that
command there. The monitor reads `herdr api snapshot` for local sessions and
the configured ix-relay Redis bus for registered leads and recent messages.
The current-pane header uses `HERDR_PANE_ID`; when Herdr does not provide that
variable or the pane is absent from the snapshot, the header says the identity
is unavailable. `r` refreshes immediately, and `q` or Escape exits.

Herdr and Redis reads run on a background thread. Each source keeps its last
good data and displays its latest error; data older than 15 seconds is marked
stale. Relay message text is shown as read-only activity and sanitized before
it reaches the terminal. Herdr sessions and registered relay leads are the
data sources; collaboration subagents that do not register in Herdr or the
relay bus are not visible as separate rows.
