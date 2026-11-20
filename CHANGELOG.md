# Release notes

## 0.3.4

- Publish the versioned `agent-spend-dashboard` Helm chart for Grafana to consume.
- Add the agent-facing procedure for configuring and troubleshooting Claude Code, Codex, and relay-watcher telemetry on Herdr machines.

## 0.3.3

- Deliver stranded mail. When a lead's session is replaced or gone and exactly one verified session holds the same `server:workspace:tab` address, its queued mail moves there, each message marked as sent to a prior session with its original time and sender. With no holder, the mail is dead-lettered and the registration pruned at the 24-hour prune age.
- Try the Claude inbox socket only for Claude leads. Codex and opencode leads go straight to herdr, and doctor no longer warns about them.
- Keep only live, registered Claude leads in the socket status; check processes outside the delivery lock, at most once per place-refresh period.
- Count wire drift per Claude Code version over a 24-hour window, with the last offending line type. Doctor reports each version as working, drifting or listed, replacing the single "captured on 2.1.283: unverified" note.
- Doctor's hook-failure check fails only for a fleet-wide burst (50 or more distinct failures in 15 minutes) or one session failing repeatedly, and names the top sessions and machines. Otherwise it warns.
- Ship each spooled hook-failure record to the bus once, by id. Records spooled by an older binary have no id and are dropped on upgrade.
- Upgrade all machines together: an older `agent-msg` rejects a forwarded message instead of showing it.

## 0.3.2

- Resolve lead addresses through a fenced relay directory in Redis instead of the sender's SSH access to the destination machine's Herdr.
- Forward directory publication authority through the production delivery adapters, and keep idle watcher budgets while scheduling directory observations.
- Add the relay doctor skill, with guarded recovery of a Codex daemon-context failure through a temporary Herdr helper tab.
- Remove obsolete OpenCode alias guidance and document each adapter's boundaries.
- Record the live fleet acceptance: cross-server round trips with luna, the native smoke run, and receipt in a Codex lead.

## 0.3.1

- Bound atomic enrollment to one Redis reply.
- Make identity diagnostics fail when enrollment is unavailable.

## 0.3.0

- Replace lead aliases with verified Herdr location identities (`server:workspace:tab`).
- Move the bus to the `ixr:v3:` key namespace. Archive the earlier namespace without deleting its keys using `scripts/archive-relay-namespace.py --prefix ixr: --output <file>`, then stage matching binaries, launcher and plugin guidance on every machine before changing the configured prefix. Earlier aliases are never imported.
- Run relay-redis as its own Redis on the hub, reached over WireGuard with one Redis user per machine. `scripts/relay-admin migrate` copies `ixr:v3:*` keys from the earlier shared Redis and keeps any key relay-redis already has.
- Reject stale interactive identities that have no matching live session, and diagnose stale Codex caller context with limited SessionStart capture.
- Resolve cold remote leads through exact saved Herdr profiles.
- Bound hook session probing to one Redis reply, and measure the drip-input deadline at hook exit.
- Observe health and socket outcomes without fixture races.

## 0.2.1

- Support Claude inbox socket delivery on macOS, including native process identity and receipt reads.
- Test relative addresses through the CLI and distinguish missing verified sender location from invalid syntax.
- Require verified sender location for shorthand addressing.
- Add message recovery guidance: confirmed Herdr fallback, join invitations, registration repair, and duplicate-send prevention.
- Provide the recovery workflow to Claude and Codex leads.
