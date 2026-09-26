// ix-relay opencode plugin (AGE-2016): connects an opencode team-lead session to
// the ix-relay message bus. It shells out to `agent-msg`, the same CLI the
// Claude Code plugin (plugins/claude) and Codex use, so all three harnesses
// share one bus and one message format.
//
// Verified against opencode 1.17.11 / @opencode-ai/plugin 1.17.11 (see
// plugins/opencode/README.md for the evidence and what's *not* provable in a
// one-shot `opencode run`):
//
//   - mid-turn: `tool.execute.after` can append text to a tool's own output,
//     which reaches the model as part of that tool's result. That's the
//     delivery point proven end-to-end: the appended text showed up in the
//     model's own stream, regardless of which tool carried it.
//   - turn end / idle: the `session.idle` event fires when a turn ends.
//     `client.session.promptAsync` can start a new turn on it. But
//     `opencode run` kills its ephemeral server the instant the idle event
//     finishes dispatching — even a pending `setTimeout` inside this
//     handler never got to fire in testing — so nothing this handler does
//     asynchronously after that point can be trusted to run, and popping the
//     inbox here would be popping messages nobody can promise to see. So the
//     idle handler NEVER pops: it only peeks the count (`agent-msg inbox
//     --count`, non-destructive) and, if nonzero, fires a short nudge
//     ("you have messages — run `agent-msg inbox` now") via
//     `client.session.promptAsync`. The pop then happens inside the model's
//     own subsequent bash call, whose output the model always sees — that
//     part doesn't depend on this plugin or on the process surviving.
//     For a session under `opencode serve` (long-lived, not torn down at
//     idle) this reliably produces a real reply; for `opencode run` the
//     nudge is queued but the run may exit before the model acts on it.
//     Otherwise-idle leads are covered by relay-watcher (AGE-2013), which
//     wakes them by typing into the opencode TUI through herdr.
//   - session start: `session.created` fires reliably and carries
//     `Session.parentID`. There is no delivery point that fires before any
//     tool call and before any turn ends, so backlog queued before a lead's
//     session starts is delivered at whichever of the above happens first.
//
// Subagent detection is deny-by-default: a session is only treated as a lead
// once `Session.parentID` has been positively confirmed absent, either from
// a `session.created`/`session.updated` event or, for a session this plugin
// has not seen an event for yet, from `client.session.get`. A session that
// can't be resolved is treated as not-a-lead, not as a lead.
//
// Delivery is bound to whichever confirmed lead session was most recently
// active in this opencode process. If more than one lead session runs under
// one process, only the one currently doing something receives ix-relay
// deliveries — this avoids two idle tabs racing to drain the same inbox.
//
// Registration only latches in once `agent-msg register` actually exits 0;
// a failure is retried on the next event instead of being silently assumed
// to have worked, and a session resumed without a fresh `session.created`
// still registers via the same opportunistic check every other handler does.
//
// AGENT_MSG_NAME is optional (AGE-2018): `agent-msg` derives a bus name from
// this pane's herdr location when it's unset, so this plugin no longer reads
// the env var itself to decide whether it's in a lead session. It asks
// `agent-msg whoami --json` once, at plugin load, which resolves the same
// way agent-msg's own hook does: the explicit env var if set, else a
// herdr-derived name, else "not a lead". Every later `agent-msg` call this
// plugin makes (register/inbox/...) re-resolves its own identity the same
// way, from the herdr env vars the child process inherits — this plugin
// never needs to pass a resolved name down to them itself.
//
// Silent when whoami resolves to "not a lead" (no explicit name and no
// herdr location to derive from), same as `agent-msg hook`. Never throws
// back into opencode's own turn: every `agent-msg` call is best-effort, and
// a failure is logged to stderr (visible in opencode's own logs, alongside
// `agent-msg log`'s own history) rather than swallowed.

import type { Plugin } from "@opencode-ai/plugin";

/** `agent-msg whoami --json`'s shape: a resolved name, or `{"lead": false}`. */
interface WhoamiReport {
  name?: string;
  source?: "explicit" | "derived";
  lead?: false;
}

/** One message as `agent-msg inbox --json` prints it. */
interface InboxMessage {
  from: string;
  to: string;
  text: string;
  sent_at_ms: number;
  rendered: string;
}

/** The `{"messages": [...], "rejected": [...]}` shape of `inbox --json`. */
interface InboxReport {
  messages: InboxMessage[];
  rejected: string[];
}

export const IxRelayPlugin: Plugin = async ({ client, $ }) => {
  // Resolves once, at plugin load, whether this process is a lead at all
  // (AGE-2018): the explicit AGENT_MSG_NAME/--as override when set, else a
  // name derived from this pane's herdr location. Not a lead session (or a
  // one-off, or not in herdr at all): stay silent, exactly like
  // `agent-msg hook` does.
  const whoamiResult = await $`agent-msg whoami --json`.nothrow().quiet();
  if (whoamiResult.exitCode !== 0) {
    console.error(
      `ix-relay: agent-msg whoami failed (exit ${whoamiResult.exitCode}): ${whoamiResult.stderr.toString().trim()}`,
    );
    return {};
  }
  let whoami: WhoamiReport;
  try {
    whoami = JSON.parse(whoamiResult.text().trim()) as WhoamiReport;
  } catch (e) {
    console.error(`ix-relay: agent-msg whoami produced unparseable output: ${(e as Error).message}`);
    return {};
  }
  if (whoami.lead === false || !whoami.name) {
    return {};
  }

  // Resolved lead/subagent status per session id, so a session is never
  // treated as a lead until its parentID has been positively confirmed
  // absent. Deny-by-default: a session with no entry here is NOT a
  // confirmed lead.
  const isLeadSession = new Map<string, boolean>();

  // Which confirmed lead session most recently did something. Delivery is
  // gated on being that session, so two idle lead sessions in one process
  // don't both try to drain the inbox.
  let activeLeadSession: string | undefined;

  // Only true once `agent-msg register` has actually exited 0. A failure
  // leaves this false so the next event retries it.
  let registrationDone = false;

  function noteSessionInfo(info: { id?: string; parentID?: string } | undefined) {
    if (info?.id) {
      isLeadSession.set(info.id, !info.parentID);
    }
  }

  /** Resolves whether `sessionID` is a confirmed lead, deny-by-default. */
  async function isConfirmedLead(sessionID: string): Promise<boolean> {
    const known = isLeadSession.get(sessionID);
    if (known !== undefined) return known;
    try {
      const res = await client.session.get({ path: { id: sessionID } });
      const parentID = (res as { data?: { parentID?: string } })?.data?.parentID;
      const error = (res as { error?: unknown })?.error;
      if (error || !(res as { data?: unknown })?.data) {
        // Unresolvable: deny, don't guess this is a lead.
        isLeadSession.set(sessionID, false);
        return false;
      }
      const isLead = !parentID;
      isLeadSession.set(sessionID, isLead);
      return isLead;
    } catch {
      isLeadSession.set(sessionID, false);
      return false;
    }
  }

  /** Marks `sessionID` as the most recently active confirmed lead. */
  function touchActive(sessionID: string) {
    activeLeadSession = sessionID;
  }

  /** Is `sessionID` still the session delivery is bound to right now? */
  function isActive(sessionID: string): boolean {
    return activeLeadSession === sessionID;
  }

  /** Registers once; retried on the next call if the last attempt failed. */
  async function ensureRegistered() {
    if (registrationDone) return;
    const result = await $`agent-msg register --kind opencode`.nothrow().quiet();
    if (result.exitCode === 0) {
      registrationDone = true;
    } else {
      console.error(
        `ix-relay: agent-msg register failed (exit ${result.exitCode}), will retry: ${result.stderr.toString().trim()}`,
      );
    }
  }

  /** Non-destructive: how many messages are pending, without popping them. */
  async function peekInboxCount(): Promise<number> {
    try {
      const result = await $`agent-msg inbox --count`.nothrow().quiet();
      if (result.exitCode !== 0) {
        console.error(
          `ix-relay: agent-msg inbox --count failed (exit ${result.exitCode}): ${result.stderr.toString().trim()}`,
        );
        return 0;
      }
      const n = Number.parseInt(result.text().trim(), 10);
      return Number.isFinite(n) ? n : 0;
    } catch (e) {
      console.error(`ix-relay: agent-msg inbox --count threw: ${(e as Error).message}`);
      return 0;
    }
  }

  /** Pops the pending inbox and returns the already-rendered `[ix-relay]` lines. */
  async function popInboxLines(): Promise<string[]> {
    // At-most-once: `inbox --json` pops outright and records nothing in flight, so
    // messages popped here are lost if opencode aborts before they are shown. The
    // hook and the watcher acknowledge their deliveries; this path needs a claim and
    // an acknowledgement too (AGE-2051).
    const result = await $`agent-msg inbox --json`.nothrow().quiet();
    if (result.exitCode !== 0) {
      console.error(
        `ix-relay: agent-msg inbox --json failed (exit ${result.exitCode}): ${result.stderr.toString().trim()}`,
      );
      return [];
    }
    const raw = result.text().trim();
    let report: InboxReport;
    try {
      report = JSON.parse(raw) as InboxReport;
    } catch (e) {
      console.error(
        `ix-relay: agent-msg inbox --json produced unparseable output: ${(e as Error).message}; raw: ${raw}`,
      );
      return [];
    }
    for (const rejected of report.rejected ?? []) {
      console.error(`ix-relay: dropped one inbox entry: ${rejected}`);
    }
    return (report.messages ?? []).map((m) => m.rendered);
  }

  return {
    event: async ({ event }) => {
      if (event.type === "session.created" || event.type === "session.updated") {
        const info = (event.properties as { info?: { id?: string; parentID?: string } })?.info;
        noteSessionInfo(info);
        if (info?.id && !info.parentID) {
          await ensureRegistered();
        }
        return;
      }

      if (event.type === "session.idle") {
        const sessionID = (event.properties as { sessionID?: string })?.sessionID;
        if (!sessionID) return;
        await ensureRegistered();
        if (!(await isConfirmedLead(sessionID))) return;
        touchActive(sessionID);
        if (!isActive(sessionID)) return;

        const count = await peekInboxCount();
        if (count === 0) return;
        const plural = count === 1 ? "message" : "messages";
        const result = await client.session.promptAsync({
          path: { id: sessionID },
          body: {
            parts: [
              {
                type: "text",
                text: `ix-relay: you have ${count} pending ${plural} from other leads — run \`agent-msg inbox\` now.`,
              },
            ],
          },
        });
        const error = (result as { error?: unknown })?.error;
        if (error) {
          console.error(`ix-relay: promptAsync nudge failed for ${sessionID}: ${JSON.stringify(error)}`);
        }
      }
    },

    "tool.execute.after": async (input, output) => {
      await ensureRegistered();
      if (!(await isConfirmedLead(input.sessionID))) return;
      touchActive(input.sessionID);
      if (!isActive(input.sessionID)) return;

      const lines = await popInboxLines();
      if (lines.length === 0) return;
      const prefix = output.output ? `${output.output}\n\n` : "";
      output.output = `${prefix}${lines.join("\n")}`;
    },
  };
};

export default IxRelayPlugin;
