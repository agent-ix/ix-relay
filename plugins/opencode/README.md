# ix-relay opencode plugin

Connects a team lead's opencode session to the ix-relay bus: incoming messages
from other leads show up in the model's context automatically.

This plugin is for **team leads only**. A subagent (task) session should not
need it installed separately — it already skips any session opencode marks as
a subagent (`Session.parentID` set) — but there's no reason to pay even the
plugin's `agent-msg` calls in a session that isn't a lead. It also skips any
session that resolves to no bus name at all: no explicit `AGENT_MSG_NAME` and
no herdr location to derive one from (AGE-2018) — see "AGENT_MSG_NAME is
optional" below.

## What was verified, and how (AGE-2016)

Checked against opencode `1.17.11` and `@opencode-ai/plugin` `1.17.11` (the
plugin API version that ships with it), installed via `npm view
@opencode-ai/plugin@1.17.11` / `npm pack` and read directly from
`dist/index.d.ts`, plus live runs of `opencode run` against a real plugin file
under `.opencode/plugin/`.

1. **Is opencode installed?** Yes: `opencode --version` → `1.17.11`
   (`opencode-ai` installed globally via npm at
   `~/.npm-global/lib/node_modules/opencode-ai`). No install step was needed
   in this environment; where it is missing, install it user-locally with
   `npm install -g opencode-ai` (no sudo) or opencode's own install script —
   both put a `opencode` binary on `PATH` without touching system
   directories.

2. **Can a JS/TS plugin inject text mid-turn, keep a turn from ending or
   start a new one, and add context on session start?**

   - **(a) inject mid-turn — yes, verified live.** The `"tool.execute.after"`
     hook receives `(input: {tool, sessionID, callID, args}, output: {title,
     output, metadata})`; `output` is mutable, and the harness sends the
     mutated `output.output` back to the model as that tool's result. A probe
     plugin that appended a marker string to `output.output` produced a
     marker that showed up verbatim in the model's own event stream on the
     very next step. This is the plugin's main delivery point, and it mirrors
     Claude Code's `PostToolUse` hook.

   - **(b) block/restart a turn — verified live, end-to-end, against
     `opencode serve`.** There is no hook that can block a turn from ending
     (nothing like Claude's `Stop` → `decision: "block"`). There *is*
     `client.session.promptAsync({path: {id}, body: {parts}})` — "create and
     send a new message to a session, start if needed, and return
     immediately" — callable from the `event` hook on `session.idle` (fires
     when a turn ends). The first version of this plugin called it with the
     *actual pending messages* and popped the inbox right there in the idle
     handler. That's wrong, and PR review caught it: `opencode run` tears
     down its ephemeral local server the instant the idle event finishes
     dispatching — confirmed by putting a bare `setTimeout` in the handler
     after the `promptAsync` call and watching it never fire even once — so
     nothing async this handler does after that point can be trusted to run,
     and a pop here is a pop nobody can promise gets seen. The fix: the idle
     handler **never pops**. It only peeks the count
     (`agent-msg inbox --count`, non-destructive) and, if nonzero, sends a
     short nudge — `"you have N pending messages — run agent-msg inbox
     now"` — checking `result.error` and logging it if set. The actual pop
     happens inside the model's own subsequent `agent-msg inbox` call, whose
     output the model always sees regardless of whether this plugin's own
     process survives. Verified end-to-end against `opencode serve` (a
     persistent server, not torn down at idle): started the server, drove an
     initial turn against it with `opencode run --attach`, sent a message
     once that turn's only tool call had completed, and polled the server's
     own `/session/{id}/message` over HTTP — the model's own follow-up
     `agent-msg inbox` output, containing the pending message, showed up
     there. Under plain `opencode run` (no persistent server) the nudge is
     still queued via `promptAsync`, but whether the model acts on it before
     the process exits isn't guaranteed the way it is under `serve`/the TUI.

   - **(c) session start — yes, verified live.** The `event` hook fires
     `session.created` reliably, with `properties.info` being a full
     `Session` object (id, `parentID`, directory, ...). The plugin registers
     the lead here. There is no delivery point that fires *before* any tool
     call and before any turn ends, so backlog queued before a session starts
     is delivered at whichever of (a) or (b) happens first — in practice,
     almost always (a), since a lead's first turn calls a tool.

   A `"chat.message"` hook also exists (`(input: {sessionID, ...}, output:
   {message, parts})`) that looks like it should let a plugin push an extra
   `Part` into the very first user message. In testing, pushing a
   well-formed `TextPart` (with `id`/`sessionID`/`messageID` set) into
   `output.parts` did not error, but the pushed text never appeared anywhere
   in that session's own event stream — so it is **not** used here; the
   plugin relies only on the two verified mechanisms above.

3. **How does opencode mark a subagent (task) session?** `Session.parentID`
   (from `@opencode-ai/sdk`'s generated `types.gen.d.ts`): unset for a lead's
   own top-level session, set to the lead's session id for a subagent's child
   session. The plugin tracks this from `session.created`/`session.updated`
   events, and resolves it via `client.session.get` for any session it sees
   in `"tool.execute.after"` or `session.idle` before an event has told it
   what that session is — the same role `agent_id` plays in Claude's hook
   JSON. This is **deny-by-default**: a session is only treated as a lead
   once its absence of `parentID` has been positively confirmed; a session
   that can't be resolved (the `client.session.get` call errors, or returns
   nothing) is treated as not-a-lead, not as a lead. The first version of
   this plugin got this backwards — an unrecognized session was allowed
   through by default — which review also caught.

4. **What if more than one lead session runs in one opencode process?**
   Delivery is bound to whichever confirmed lead session was most recently
   active. `tool.execute.after` and `session.idle` both re-check, after
   their own `agent-msg` round trip, that their session is still the most
   recently active one before doing anything — so two idle lead sessions
   under one process don't both try to drain the same inbox, and a message
   surfaces in whichever session you're actually looking at.

## Install

1. **Build `agent-msg` and put it on `PATH`** — same as the Claude plugin
   (see `plugins/claude/README.md`):

   ```bash
   cargo install --path crates/agent-msg
   ```

2. **`AGENT_MSG_NAME` is optional (AGE-2018).** A lead running inside a
   herdr-managed pane gets a bus name derived from where it sits in herdr —
   see the main [README.md](../../README.md#automatic-bus-names) for the
   naming rule. Set `AGENT_MSG_NAME` explicitly in the environment the
   opencode session runs in only to override that (it's never rewritten).
   Leave it unset for any session that is not a team lead, or one outside
   herdr; the plugin stays silent either way. It resolves its own identity
   once, at load, via `agent-msg whoami --json` (see "What was verified"
   below) — it no longer reads `AGENT_MSG_NAME` directly itself.

3. **Set `~/.ix/relay/config.toml` if this machine needs different settings**
   — Redis isn't reachable at the default address (`redis://127.0.0.1:6379`),
   the key prefix needs to differ, or the machine's hostname needs fixing.
   This is a machine-level file, not per-session (same as the Claude plugin,
   see `plugins/claude/README.md`): set it up once, and every lead session on
   the machine — Claude, Codex or opencode — picks it up.

   ```toml
   url = "redis://127.0.0.1:6380"
   machine = "node-b"
   ```

   See the main [README.md](../../README.md#configuration) for the full key
   list and precedence order. Most machines reach the bus through an SSH
   tunnel at the default address and need no file at all.

4. **Install the plugin file.** opencode auto-loads every file under a
   project's `.opencode/plugin/` directory, or a global one under
   `~/.config/opencode/plugin/`. Either copy or symlink it in:

   ```bash
   mkdir -p .opencode/plugin
   ln -s /path/to/ix-relay/plugins/opencode/ix-relay.ts .opencode/plugin/ix-relay.ts
   ```

   or reference it explicitly from `opencode.json` without copying anything:

   ```json
   {
     "plugin": ["/path/to/ix-relay/plugins/opencode/ix-relay.ts"]
   }
   ```

5. **Give the lead the bus guidance.** opencode combines `AGENTS.md` with any
   files listed under `instructions` in `opencode.json`. Point it at
   `plugins/opencode/AGENTS.md` (the same content as the Claude plugin's
   `skills/msg/SKILL.md`, in opencode's own instructions format) instead
   of copying it in:

   ```json
   {
     "instructions": ["/path/to/ix-relay/plugins/opencode/AGENTS.md"]
   }
   ```

## Verifying it's working

```bash
agent-msg who      # see other registered leads
agent-msg log       # recent bus history
```

If `agent-msg who` only shows leads you expect and `agent-msg log` shows sends
you've made land, the plugin is reaching the same Redis your CLI does.

`scripts/live-test-opencode-plugin.sh` runs a fuller, automated version of
this against a throwaway Redis and a throwaway opencode config dir, including
the idle-nudge path (against `opencode serve`) and an exchange with a live
Claude Code lead in both directions.

## Idle leads

A lead sitting completely idle (no turn running at all, not even one the
`session.idle` nudge could ride) isn't reachable by this plugin at all — there
is no opencode hook that fires without a turn in flight. That's covered by
`relay-watcher` (AGE-2013), which polls registered leads and wakes an idle one
by typing into its herdr-managed pane with `herdr agent prompt`, the same way
it does for Claude and Codex leads.
