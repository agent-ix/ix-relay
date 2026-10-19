# Codex daemon recovery scenarios

## A restart still attached to the shared daemon

An existing shell can retain its old PATH after the launcher is installed.
The process runs an NVM/npm Codex without `--no-daemon`, tools inherit a stale
pane, and Herdr has no genuine session association. This is a launch-context
failure, even if Redis and the watchers are healthy.

Use the installed launcher's absolute path and pass `--no-daemon` explicitly:

```bash
~/.local/bin/codex --no-daemon resume <verified-session-to-resume>
```

Do not stop the shared daemon: other active leads may depend on it.

## Identify the original terminal

Read `herdr --skill` unless already loaded. Require `HERDR_ENV=1`; inspect the
installed `herdr` command help. Use `herdr pane current --current`,
`herdr agent get <pane>`, and `herdr pane process-info --pane <pane>`.

Use the verified caller's live pane, workspace, tab, terminal ID, genuine
harness session, foreground Codex PID, cwd, and process argv. Capture only these
fields, the diagnosed problem, authorization, and a unique repair ID in a
private handoff file. Preserve the original session's environment and cwd.

Stale inherited pane IDs or a matching title are insufficient. If the API caller
has no verified association or demonstrable terminal-process ancestry, obtain
the operator's explicit target pane and resume-session confirmation. Verify the
confirmed terminal's foreground Codex resume argv and PID against that handoff.
Do not treat the helper's new session ID as the original resume ID.

## Temporary helper tab

Create a tab in the original workspace, with the same cwd and `--no-focus`:

```bash
herdr tab create --workspace <verified-workspace-id> --cwd <verified-cwd> --label relay-doctor --no-focus
herdr agent start <unique-helper-name> --kind codex --pane <returned-root-pane-id> -- --no-daemon
```

Read tab and pane IDs from the creation response. The installed CLI currently
has no tab insertion/reorder option; use the new same-workspace tab without
moving existing work to force adjacency. Keep the original pane open.

Give the helper the absolute handoff path and this reference's absolute path
using `herdr agent prompt <helper> <handoff-instructions>`, without `--wait`.
Use Herdr and the handoff file for recovery coordination. The helper can be
enrolled automatically as a temporary lead; never manually register it or use
a guessed relay sender to control the restart.

The original agent must then finish its turn, report that recovery is handed
off, and yield. Do not synchronously wait for a helper that is waiting for the
original agent to become idle.

## Helper procedure

1. Read the handoff. Confirm explicit restart authorization, original target
   terminal ID, foreground PID/argv, resume ID, cwd, and owned helper tab ID.
   Refuse a replaced process or a moved/closed target. Never retarget silently.
2. Wait for the original agent to become `idle` or `done`. Use bounded Herdr
   waits and recheck state and process identity immediately before exit. If it
   is `working`, wait; if `blocked` or `unknown`, inspect and report. Do not
   send Ctrl+C, kill a process, or answer a trust/approval dialog automatically.
3. Send `ctrl+d` to the verified original agent. Wait for its foreground to
   become the existing interactive shell. If exit fails, leave the pane and
   handoff intact and report the failure rather than escalating to a kill.
4. Run the absolute installed launcher with explicit `--no-daemon resume`
   and the verified resume ID in the original pane. Quote argv safely; preserve
   cwd and environment. Never clear `AGENT_MSG_NAME`, inject the helper's pane
   variables, or fabricate registration. Verify the new process has the flag.
5. After the original Codex is ready, submit one recovery-verification prompt
   using `herdr agent prompt <original-pane>`. SessionStart can occur on this
   first prompt rather than the welcome screen. Ask the resumed agent to record
   `agent-msg whoami --json`, its genuine `CODEX_THREAD_ID`, and current Herdr
   session association in the private receipt file. Resume may produce a new
   harness session ID; verify that exact genuine ID, not a guessed old ID.
6. Require the expected three-part location and matching live Herdr association.
   Have the resumed lead send one unique self-test with `agent-msg send :tab`
   using its actual tab label. Verify receipt through the normal receiving hook;
   queue success, an empty queue, and Redis health alone are not receipt proof.
   Do not manually pop the inbox to hide a receiving-hook failure or retry an
   ambiguous timeout. Record queued, received, and acknowledged separately.
7. Write the outcome to the handoff/receipt file before cleanup. After verified
   recovery, close only the helper tab with `herdr tab close <owned-helper-tab>`.
   This ends the helper. The resumed original agent reports the result to the
   operator. On failure or approval UI, retain the helper tab and report its
   exact ID and blocker; ask before answering the dialog.

## Healthy launcher, missing registration

If `--no-daemon` is already present, investigate the genuine SessionStart and
installed hooks before restarting. Herdr's SessionStart helper must run before
`agent-msg hook codex`; hooks must be enabled and trusted. Capture only event,
session ID, transcript path and Herdr context when necessary. Never synthesize
the event. A nearby terminal or stale registry entry cannot establish identity.
