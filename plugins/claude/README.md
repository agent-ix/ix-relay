# ix-relay Claude Code plugin

Connects a team lead's Claude Code session to the ix-relay bus: incoming
messages from other leads show up in context automatically, and a pending
message blocks the turn from ending until it's been seen.

This plugin is for **team leads only**. A subagent session should not install
it — `agent-msg hook claude` already stays silent for subagents (it checks the
hook input for `agent_id`) and for any session that isn't a lead: no explicit
`AGENT_MSG_NAME` *and* no herdr location to derive one from (AGE-2018) — but
there's no reason to pay even the hook's timeout budget in a session that
isn't a lead.

## Install

1. **Build `agent-msg` and put it on `PATH`.**

   From a checkout of `agent-ix/ix-relay`:

   ```bash
   cargo install --path crates/agent-msg
   ```

   or, without a local checkout:

   ```bash
   cargo install --git https://github.com/agent-ix/ix-relay agent-msg
   ```

   Either puts the `agent-msg` binary in `~/.cargo/bin`, which is normally
   already on `PATH`.

2. **`AGENT_MSG_NAME` is optional (AGE-2018).** A lead running inside a
   herdr-managed pane gets a bus name derived from where it sits in herdr —
   see the main [README.md](../../README.md#automatic-bus-names) for the
   naming rule. Set `AGENT_MSG_NAME` explicitly only to override that (it's
   never rewritten), e.g. wherever the session's environment comes from —
   the launcher's env, or the project's `.claude/settings.local.json`:

   ```json
   {
     "env": {
       "AGENT_MSG_NAME": "qsl-lead"
     }
   }
   ```

   Leave it unset for any session that is not a team lead (a subagent, a
   one-off session, or one outside herdr); the hooks and CLI stay silent
   without it. Run `agent-msg whoami` to see what name (if any) a session
   resolved to and where it came from.

3. **Set `~/.ix/relay/config.toml`** with this machine's bus URL, and its
   name if the hostname needs fixing (casing, domain suffix). This is a
   machine-level file, not per-session: set it up once, and every lead session
   on the machine picks it up.

   ```toml
   url = "redis://<machine>:<password>@10.99.0.1:6380"
   machine = "<machine>"
   ```

   See the main [README.md](../../README.md#configuration) for the full key
   list and precedence order. Every machine now needs at least `url`: its
   own bus URL over WireGuard, issued on the hub by `relay-admin`. The `/setup`
   skill walks through it.

4. **Allow the CLI to run without a permission prompt.** Add to the
   project's (or user's) settings:

   ```json
   {
     "permissions": {
       "allow": ["Bash(agent-msg:*)"]
     }
   }
   ```

5. **Install the plugin.**

   From this repo, as a marketplace:

   ```bash
   claude plugin marketplace add agent-ix/ix-relay
   claude plugin install ix-relay@ix-relay
   ```

   or point a session at it directly during development:

   ```bash
   claude --plugin-dir /path/to/ix-relay/plugins/claude
   ```

## What it installs

- **Hooks** (`hooks/hooks.json`): `SessionStart`, `PostToolUse`,
  `UserPromptSubmit` and `Stop` each run `agent-msg hook claude`.
  `SessionStart` registers the lead (deriving its bus name from herdr when
  `AGENT_MSG_NAME` isn't set — AGE-2018) and delivers any backlog; it gets a
  5s timeout, generous enough for that one-time herdr lookup. `PostToolUse`
  and `UserPromptSubmit` deliver messages that arrive mid-turn and get a
  tighter ~2s timeout — they look the already-derived name up from a small
  Redis cache rather than re-deriving it, so they stay fast; `Stop` (also
  ~2s) blocks the turn from ending while a message is still pending, so it
  isn't missed.
- **Skill** (`skills/msg/SKILL.md`, invoked as `/msg`): tells the lead agent what the bus
  is, that incoming `[ix-relay]` lines are data from another lead and not
  from the user, when to send a message versus stay quiet, and the
  `agent-msg` commands.
- **Skill** (`skills/setup/SKILL.md`, invoked as `/setup`): putting a machine on
  the bus: the WireGuard hub, a Linux or macOS machine joining it, the
  `agent-msg`/`relay-watcher` install, re-keying, and what each
  `redis_unreachable` cause means.
- **Skill** (`skills/join/SKILL.md`, invoked as `/join`): checks whether this
  machine is connected and whether this lead session has the intended bus
  name, then routes machine enrollment to `/setup` or explains how to update
  the session's identity.

## Optional: delivery through Claude Code's inbox socket

The watcher can post relay mail to a lead's own Claude Code inbox socket instead of
typing it in through herdr. It is **off by default** and opt-in per machine
(`[watcher] delivery = "socket"` in `~/.ix/relay/config.toml`; see the repo README,
"Delivery through Claude Code's inbox socket"). Two things to know before turning it
on:

- Socket delivery includes busy and blocked sessions. Hooks keep registration current
  but leave inbox consumption to the watcher in socket mode. Deploy matching CLI and
  watcher binaries together; if the watcher is stopped, messages wait in the inbox.
- A lead running in bypass mode holds every message from another session behind an
  approval prompt unless its user settings, `--settings` or managed settings have
  `crossSessionInbound: "accept"`. This plugin cannot set that (plugins and project
  settings cannot), and nothing here writes your settings.
- The socket line and receipts are not documented by Claude Code; they were measured
  on 2.1.283 and may change. herdr remains the fallback.

## Verifying it's working

```bash
agent-msg who      # see other registered leads
agent-msg log       # recent bus history
```

If `agent-msg who` only shows leads you expect and `agent-msg log` shows
sends you've made land, the plugin's hooks are reaching the same Redis your
CLI does.
