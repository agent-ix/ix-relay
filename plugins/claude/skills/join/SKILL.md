---
name: join
description: Use when a lead needs to join the ix-relay bus, check whether this session is connected, or correct the name this session registers under.
---

# Join or update your ix-relay identity

Use this skill for two related tasks: connecting this machine to the bus, and
making sure this lead session registers under the intended name and has a
verified location for addressing. Machine
connectivity and a session's bus name are separate settings.

## Check this session first

Run these read-only checks:

```bash
agent-msg doctor
agent-msg whoami
```

`doctor` reports whether this machine can reach Redis and whether its watcher
is healthy. `whoami` reports the current session's name, how it was selected,
and any location alias. If the Redis check fails or the machine has no bus URL,
continue with the `/setup` skill. It covers machine enrollment, WireGuard,
credential delivery, binaries, and watcher setup. Do not ask the user to paste
a bus URL or private key into chat; the bus URL is a credential and the private
WireGuard key must stay on its machine.

## Update this session's name

Names are session-scoped. The shared `~/.ix/relay/config.toml` cannot set a
lead's name; it configures the machine. First explain the current `whoami`
result and ask what name the lead wants other leads to use if the intended name
isn't already clear.

- `AGENT_MSG_NAME`, when set in this session's environment, is the explicit
  name and takes priority over automatic naming. It is never overwritten by
  the relay. Update it in the environment/launcher that starts this session,
  then restart the session for the new value to be inherited.
- Without `AGENT_MSG_NAME`, Claude Code's latest `/rename` title determines
  the name when present. Use Claude Code `/rename` to change it; the new name
  is picked up on the next prompt.
- If neither applies, the name is derived from this session's herdr location.
  Move/rename the pane or workspace as appropriate, or set `AGENT_MSG_NAME`
  explicitly if the lead needs a stable chosen name.

After any change, run `agent-msg whoami` again. When connected, the next hook
registers the resolved name and keeps the old herdr location as an alias where
applicable. For a one-off refresh, `agent-msg register` re-registers this
session's current resolved identity; it does not change the name.

## Repair session identity and location

A bus name or successful manual registration does not prove a verified Herdr
location. Relative targets (`:tab`, `:workspace:tab`) need that location; full
addresses also need the recipient's verified registration.

If `whoami` shows a registry fallback, or `who`/`doctor` shows an unverified
location, check this session's actual Herdr context and its own session ID.
Read the installed Herdr skill before inspecting or controlling panes. Compare
`herdr agent get <this-session-pane>` with the relay registration. Both must
identify the same session before claiming that location is verified. Never
substitute the focused pane, another session's ID, or a fabricated ID.

Enable the harness's ix-relay hooks and Herdr session integration, then refresh
registration from this session's genuine hook event. If a session was started
before integration was enabled, report that it may need a hook refresh or a
session restart; do not restart active work without authorization. Manual
`agent-msg register` may establish a bus name without session identity and
must not be presented as fixing shorthand. Preserve `AGENT_MSG_NAME`.

If Redis answers but location remains unverified, report both facts separately
and use a confirmed bus name for delivery while repairing the integration. If
the lead is unreachable on relay, the sender can use the
[message recovery workflow](../msg/references/delivery-recovery.md) to contact
this session and invite it to join.

## Finish

When `doctor` shows Redis answers and `whoami` shows the intended identity,
report the bus name, its source, and watcher status. If Redis is reachable but
the session is not registered, run `agent-msg register` and check `agent-msg
who` for the lead. Never claim the lead has joined based only on an installed
plugin or a configured name. Report whether its session location is verified;
do not claim relative addressing works until it is.
