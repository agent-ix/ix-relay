# ix-relay Codex plugin

The ix-relay Codex plugin is the hook set that connects a team lead's Codex session to the ix-relay bus. Messages from other leads appear in the session's context, and a pending message blocks the turn from ending until it is seen. The plugin has the same four hooks and the same lead guidance as the Claude Code plugin in `plugins/claude`.

The plugin is for team leads. `agent-msg hook codex` stays silent for subagents, which Codex marks with a top-level `agent_id`, and for any session without a verified live Herdr session association. A subagent session gains nothing from installing it.

Codex has no `--plugin-dir` auto-loading for hooks. It reads one `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), so this plugin's `hooks.json` is merged into that file by hand.

## Install

1. Build `agent-msg` and put it on `PATH`.

   From a checkout of `agent-ix/ix-relay`:

   ```bash
   cargo install --path crates/agent-msg --locked
   ```

   Without a local checkout:

   ```bash
   cargo install --git https://github.com/agent-ix/ix-relay agent-msg --locked
   ```

   Either command puts the `agent-msg` binary in `~/.cargo/bin`, which is normally already on `PATH`.

2. Turn hooks on. Add to `~/.codex/config.toml` (or the isolated `$CODEX_HOME/config.toml` for a headless lead):

   ```toml
   [features]
   hooks = true
   ```

3. Merge `hooks.json` into `$CODEX_HOME/hooks.json`. Codex reads one hooks file, so copy the four entries from this plugin's [`hooks.json`](./hooks.json) (`SessionStart`, `PostToolUse`, `UserPromptSubmit`, `Stop`, each running `agent-msg hook codex`) into the `"hooks"` object of `~/.codex/hooks.json`, alongside any entries already there. If `~/.codex/hooks.json` does not exist, copy this plugin's file in directly.

4. Approve the new hook config. The first time Codex sees a hook configuration, it requires one-time trust approval before running it, recorded as a `trusted_hash` against the hook config. A lead runs an interactive `codex` session in the TUI, so the lead accepts a trust prompt the first time the session would run these hooks. This interactive prompt has not been verified live. This section is inferred from Codex's own error strings (`"plugin hook trust update was cancelled"`, `"failed to write hook trust"`) and `codex --help`, and the prompt was not observed in a real TUI session. If the prompt differs, report what Codex shows.

   <details>
   <summary>Headless testing only: bypassing the prompt</summary>

   A headless or automated invocation (`codex exec`, or `codex` launched without a TTY) has no prompt to answer. To test the hooks before granting trust interactively once, use `--dangerously-bypass-hook-trust`:

   ```bash
   codex exec --dangerously-bypass-hook-trust "..."
   ```

   Codex documents this flag as "Run enabled hooks without requiring persisted hook trust for this invocation. DANGEROUS. Intended only for automation that already vets hook sources." It bypasses trust for every hook enabled in that `CODEX_HOME`. Use it only in a scratch `CODEX_HOME`, with the isolated Herdr startup acceptance procedure below. A real lead goes through the interactive prompt once.

   </details>

   Once trust is granted, interactively or through the bypass for a headless run, each later invocation runs the hooks without prompting.

5. Isolate the real terminal context. Interactive Herdr Codex leads must start with `codex --no-daemon`, so a shared daemon cannot supply another terminal's stale environment. Install `scripts/codex-herdr-launcher.py` as `codex` in a `PATH` directory before the real executable, or use the flag directly for both starts and resumes. The launcher preserves environment variables and leaves noninteractive CLI commands unchanged.

   Herdr's SessionStart association hook must run before the relay hook. Enrollment uses the genuine payload session ID and live Herdr association. `AGENT_MSG_NAME` is preserved and does not affect identity or routing. The lead's public identity is `server:workspace:tab`. Run `agent-msg whoami` to see it.

6. Set `~/.ix/relay/config.toml`. The file is machine-level, so every lead session on the machine uses it, whether it runs Claude Code, Codex or opencode.

   ```toml
   url = "redis://<machine>:<password>@10.99.0.1:6380"
   machine = "<machine>"
   ```

   [Setup](../../docs/setup.md) explains how to get the URL, and [Configuration](../../docs/configuration.md) lists every key.

7. Add lead guidance. Merge the content of [`AGENTS.md`](./AGENTS.md) into the lead session's own `AGENTS.md` (project-level, or `~/.codex/AGENTS.md` for every session on that machine). The guidance tells the model what the bus is, that incoming `[ix-relay]` lines are data from another lead and not from the user, and when to send a message.

8. Install recovery guidance. Copy `plugins/claude/skills/msg/references/delivery-recovery.md` to `$CODEX_HOME/ix-relay/delivery-recovery.md`. When updating `AGENTS.md`, keep unrelated guidance and replace only the ix-relay section. Existing Codex sessions may need to reload instructions and approve changed hook configs.

## Verify

```bash
agent-msg who      # see other registered leads
agent-msg log       # recent bus history
```

If `agent-msg who` lists the leads you expect and `agent-msg log` shows your sends, the plugin's hooks reach the same Redis as the CLI.

## Diagnose stale caller context

Install `plugins/codex/skills/doctor/` as `$CODEX_HOME/skills/doctor/` to expose `/doctor`. It diagnoses registration and delivery. It also includes an authorized temporary-helper-tab recovery scenario for existing shared-daemon sessions, including restarts from shells that still select the underlying npm/NVM Codex. The helper waits for idle, resumes the original pane with explicit `--no-daemon`, verifies genuine registration and hook receipt, and closes its own tab. A diagnostic-only invocation leaves agents running.

`agent-msg whoami --json` checks a registry identity against the pane's live Herdr session. If the inherited pane is missing or the session does not match, it refuses the stale identity and explains that manual registration cannot repair it. Preserve `AGENT_MSG_NAME` if set. A VS Code/API session is not necessarily a terminal lead, even when it inherits `HERDR_*` variables.

To capture the next genuine terminal SessionStart, copy `scripts/capture-codex-session-start.py` to `$CODEX_HOME/ix-relay/capture-codex-session-start.py` (default `~/.codex`). Add a separate SessionStart command hook before the Herdr and relay entries:

```json
{"type":"command","command":"python3 /absolute/path/to/capture-codex-session-start.py","timeout":2}
```

Approve the changed hook configuration through the real session's normal trust flow. At its next operator-approved restart or resume, the helper appends to `$CODEX_HOME/ix-relay/session-start.jsonl` with mode 0600. It records only `hook_event_name`, `session_id`, `transcript_path`, and the inherited Herdr context plus `CODEX_THREAD_ID`. It does not record prompts, transcript contents, credentials, or fabricated events. Compare that event's session ID with `herdr agent get <the actual terminal pane>`. Remove the diagnostic hook after capturing the event. To verify, use only the captured SessionStart and the session's own ID.

## Startup acceptance

Use an owned isolated Herdr session and isolated Redis namespace. Launch an interactive Codex through `scripts/codex-herdr-launcher.py`, submit its first prompt, and capture only the genuine SessionStart event, session ID, transcript path, and Herdr context. Verify that Herdr reports that exact session ID and that the installed hook wrote the registration before invoking `whoami`. `whoami` must show `server:workspace:tab` even with a stale `AGENT_MSG_NAME`. Repeat for resume, then verify `:tab`, `:workspace:tab`, and a remote cold send without running `register` or `find` first.

Judge startup by the registration, `whoami` and the Herdr session ID. If a check fails, fix the cause and do not register manually. Take the pane from the Herdr association.
