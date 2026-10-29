# ix-relay Codex plugin

Connects a team lead's Codex session to the ix-relay bus: incoming messages
from other leads show up in context automatically, and a pending message
blocks the turn from ending until it's been seen. It has the same four
hooks and the same lead guidance as the Claude Code plugin in `plugins/claude`.

This plugin is for **team leads only**. A subagent session should not use it
— `agent-msg hook codex` already stays silent for subagent calls (Codex marks
them with a top-level `agent_id`, exactly like Claude Code's subagent marker)
and for sessions without a verified live Herdr session association.

Codex has no `--plugin-dir` auto-loading for hooks. It reads one
`$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), so this plugin's
`hooks.json` is merged into that file by hand.

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
   write hook trust"`) and `codex --help`. The prompt was not observed
   in a real TUI session. If it doesn't match
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
   trust for **every** hook enabled in that `CODEX_HOME`, including
   hooks other than this plugin's: anything else configured in the same `hooks.json` runs
   unreviewed too. Reach for it in an isolated, scratch `CODEX_HOME` for
   testing (using the isolated Herdr startup acceptance procedure below). A real lead goes through the
   interactive prompt once.

   </details>

   Once trust is granted (interactively, or via the bypass for a headless
   run), each subsequent invocation runs the hooks without prompting again.

5. **Isolate the real terminal context.** Interactive Herdr Codex leads must
   start with `codex --no-daemon` so a shared daemon cannot supply another
   terminal's stale environment. Install `scripts/codex-herdr-launcher.py` as
   `codex` in a PATH directory before the real executable, or use the flag
   directly for both starts and resumes. The launcher preserves environment
   variables and leaves noninteractive CLI commands unchanged.

   Herdr's SessionStart association hook must run before the relay hook.
   Enrollment uses the genuine payload session ID and live Herdr association.
   `AGENT_MSG_NAME` is preserved but ignored for identity and routing. The
   lead's public identity is always `server:workspace:tab`. Run
   `agent-msg whoami` to verify it.

6. **Set `~/.ix/relay/config.toml`.** The machine's bus URL goes in this
   machine-level file, and every lead session on the machine uses it, whether
   it runs Claude Code, Codex or opencode.

   ```toml
   url = "redis://<machine>:<password>@10.99.0.1:6380"
   machine = "<machine>"
   ```

   [Setup](../../docs/setup.md) explains how to get the URL, and
   [Configuration](../../docs/configuration.md) lists every key.

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

## Diagnose stale caller context

Install `plugins/codex/skills/doctor/` as `$CODEX_HOME/skills/doctor/` to expose
`/doctor`. It diagnoses registration and delivery and includes an authorized
temporary-helper-tab recovery scenario for existing shared-daemon sessions,
including restarts from shells that still select the underlying npm/NVM Codex.
The helper waits for idle, resumes the original pane with explicit
`--no-daemon`, verifies genuine registration and hook receipt, and closes its
own tab. A diagnostic-only invocation leaves agents running.

`agent-msg whoami --json` checks a registry identity against the pane's live
Herdr session. If the inherited pane is missing or the session does not match,
it refuses the stale identity and explains that manual registration cannot
repair it. Preserve `AGENT_MSG_NAME` if set. A VS Code/API session is not
necessarily a terminal lead, even when it inherits `HERDR_*` variables.

To capture the next genuine terminal SessionStart, copy
`scripts/capture-codex-session-start.py` to
`$CODEX_HOME/ix-relay/capture-codex-session-start.py` (default `~/.codex`). Add a
separate SessionStart command hook before the Herdr and relay entries:

```json
{"type":"command","command":"python3 /absolute/path/to/capture-codex-session-start.py","timeout":2}
```

Approve the changed hook configuration through the real session's normal trust
flow. At its next operator-approved restart/resume, the helper appends to
`$CODEX_HOME/ix-relay/session-start.jsonl` with mode 0600. It records only
`hook_event_name`, `session_id`, `transcript_path`, and the inherited Herdr
context plus `CODEX_THREAD_ID`; it does not record prompts, transcript contents,
credentials, or fabricated events. Compare that event's session ID with
`herdr agent get <the actual terminal pane>`. Remove the diagnostic hook after
capturing the event. Never synthesize a SessionStart or substitute another
session's ID to make verification pass.

## Startup acceptance

Use an owned isolated Herdr session and isolated Redis namespace. Launch an
interactive Codex through `scripts/codex-herdr-launcher.py`, submit its first
prompt, and capture only the genuine SessionStart event, session ID, transcript
path, and Herdr context. Verify that Herdr reports that exact session ID and that
registration was written by the installed hook before invoking `whoami`.
`whoami` must show `server:workspace:tab` even with a stale `AGENT_MSG_NAME`.
Repeat for resume, then verify `:tab`, `:workspace:tab`, and a remote cold send
without running `register` or `find` first. Never infer a successful startup from
the welcome screen, manually register to hide a failure, or guess a pane from
focus. The pre-0.3 alias-based live drivers were removed; their historical
results do not verify this contract.
