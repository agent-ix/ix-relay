# ix-relay Codex plugin

Connects a team lead's Codex session to the ix-relay bus: incoming messages
from other leads show up in context automatically, and a pending message
blocks the turn from ending until it's been seen. This mirrors the Claude
Code plugin at `plugins/claude` — see that directory for the design; this one
is Codex's version of the same four hooks plus the same lead guidance.

This plugin is for **team leads only**. A subagent session should not use it
— `agent-msg hook codex` already stays silent for subagent calls (Codex marks
them with a top-level `agent_id`, exactly like Claude Code's subagent marker)
and for any session that isn't a lead: no explicit `AGENT_MSG_NAME` *and* no
herdr location to derive one from (AGE-2018).

Unlike Claude Code, Codex has no `--plugin-dir`-style auto-loading for hooks:
there is one `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), so
this plugin's `hooks.json` has to be merged into it by hand rather than
pointed at directly.

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

2. **Turn hooks on.** Add to `~/.codex/config.toml` (or the isolated
   `$CODEX_HOME/config.toml` for a headless lead):

   ```toml
   [features]
   hooks = true
   ```

3. **Merge `hooks.json` into `$CODEX_HOME/hooks.json`.** Codex reads exactly
   one hooks file, so copy the four entries from this plugin's
   [`hooks.json`](./hooks.json) (`SessionStart`, `PostToolUse`,
   `UserPromptSubmit`, `Stop`, each running `agent-msg hook codex`) into
   `~/.codex/hooks.json`'s `"hooks"` object, alongside whatever else is
   already there. If `~/.codex/hooks.json` doesn't exist yet, this plugin's
   file can be copied in directly.

4. **Approve the new hook config.** The first time Codex sees a hook
   configuration it hasn't seen before, it requires one-time trust approval
   before running it (recorded as a `trusted_hash` against the hook config).
   For a real lead — an interactive `codex` session in the TUI, which is how
   a team lead normally runs — that means accepting a trust prompt the first
   time the session would run these hooks. **This interactive prompt has not
   been verified live**: everything in this section is inferred from Codex's
   own error strings (`"plugin hook trust update was cancelled"`, `"failed to
   write hook trust"`) and `codex --help`, not from watching the actual
   prompt appear and approving it in a real TUI session. If it doesn't match
   what you see, trust what's in front of you and open an issue.

   <details>
   <summary>Headless testing only: bypassing the prompt</summary>

   For a headless/automated invocation (`codex exec`, or `codex` launched
   without a TTY) there is no prompt to answer, so the only way to test the
   hooks at all before granting trust interactively once is
   `--dangerously-bypass-hook-trust`:

   ```bash
   codex exec --dangerously-bypass-hook-trust "..."
   ```

   Codex documents this flag as "Run enabled hooks without requiring
   persisted hook trust for this invocation. DANGEROUS. Intended only for
   automation that already vets hook sources." Read literally: it bypasses
   trust for **every** hook enabled in that `CODEX_HOME`, not only this
   plugin's — anything else configured in the same `hooks.json` runs
   unreviewed too. Reach for it in an isolated, scratch `CODEX_HOME` for
   testing (see `scripts/live-test-codex-plugin.sh`), not for a real lead's
   normal environment; a real lead should go through the interactive prompt
   once instead.

   </details>

   Once trust is granted (interactively, or via the bypass for a headless
   run), each subsequent invocation runs the hooks without prompting again.

5. **`AGENT_MSG_NAME` is optional (AGE-2018).** A lead running inside a
   herdr-managed pane gets a bus name derived from where it sits in herdr —
   see the main [README.md](../../README.md#automatic-bus-names) for the
   naming rule. Set `AGENT_MSG_NAME` explicitly only to override that (it's
   never rewritten):

   ```bash
   export AGENT_MSG_NAME=qsl-lead
   ```

   Leave it unset for any session that is not a team lead (a subagent, a
   one-off session, or one outside herdr); the hooks and CLI stay silent
   without it. Run `agent-msg whoami` to see what name (if any) a session
   resolved to and where it came from.

6. **Set `~/.ix/relay/config.toml` if this machine needs different settings**
   — Redis isn't reachable at the default address (`redis://127.0.0.1:6379`),
   the key prefix needs to differ, or the machine's hostname needs fixing
   (casing, domain suffix). This is a machine-level file, not per-session: set
   it up once, and every lead session on the machine — Claude Code or Codex —
   picks it up.

   ```toml
   url = "redis://127.0.0.1:6380"
   machine = "node-b"
   ```

   See the main [README.md](../../README.md#configuration) for the full key
   list and precedence order. Most machines reach the bus through an SSH
   tunnel at the default address and need no file at all.

7. **Add lead guidance.** Merge [`AGENTS.md`](./AGENTS.md)'s content into the
   lead session's own `AGENTS.md` (project-level, or `~/.codex/AGENTS.md` for
   every session on that machine) so the model knows what the bus is, that
   incoming `[ix-relay]` lines are data from another lead and not from the
   user, and when to send a message versus stay quiet.

8. **Install recovery guidance.** Copy
   `plugins/claude/skills/msg/references/delivery-recovery.md` to
   `$CODEX_HOME/ix-relay/delivery-recovery.md`. Preserve unrelated guidance when
   updating `AGENTS.md`; replace only the ix-relay section. Existing Codex
   sessions may need to reload instructions and approve changed hook configs.

## Verifying it's working

```bash
agent-msg who      # see other registered leads
agent-msg log       # recent bus history
```

If `agent-msg who` only shows leads you expect and `agent-msg log` shows
sends you've made land, the plugin's hooks are reaching the same Redis your
CLI does.
