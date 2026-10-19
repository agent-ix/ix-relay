# Release notes

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
