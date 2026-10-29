# How it works

This page describes how ix-relay resolves addresses, delivers messages to each kind of lead, and recovers from failures. The [README](../README.md) has the short version.

## Identity and enrollment

A lead's identity is the `server:workspace:tab` of its Herdr tab. The relay accepts it only when the harness session that is calling matches a live Herdr session in that tab. Names, nicknames, `/rename` titles, raw pane IDs and `AGENT_MSG_NAME` do not establish identity. `AGENT_MSG_NAME` is left unchanged in the environment and is ignored for routing.

Enrollment is automatic:

- The `SessionStart` hook registers the genuine session with its verified location.
- The owning machine's watcher observes live Herdr sessions and registers an absent recipient on first contact.
- `agent-msg whoami` prints the verified address, or `not a lead` for a session that cannot be verified.

A session queue belongs to the session. Renaming or moving a tab changes the address, and the session keeps its queue. A new session in a reused pane does not inherit the previous session's mail.

Interactive Codex leads in Herdr start with `codex --no-daemon`, installed through `scripts/codex-herdr-launcher.py`. The shared Codex daemon would otherwise supply the environment of the terminal that started it. Herdr's `SessionStart` helper must run before the relay hook.

## Directory and address resolution

Each watcher publishes a complete observation of its own machine's Herdr tabs to Redis. The observation lists every verified lead binding, including the empty and ambiguous cases. A sender resolves an address from Redis alone and needs no SSH access or Herdr profile for the destination machine.

Publication follows these rules:

- A watcher takes an ordered Redis ticket before it reads Herdr, and publication accepts only the latest ticket, once. This fences competing readers and delayed publications.
- A successful publication holds a 90-second lease on the Redis clock. A watcher refreshes it about every 30 seconds.
- A failed or incomplete read cannot renew the lease and cannot publish absence.
- A replayed or delayed observation cannot restore a binding that a newer observation removed.

A send expands shortcuts with the sender's verified location, then reads the destination machine's snapshot:

| Snapshot state for the address | Result |
|---|---|
| Fresh, one session | The message is queued for that session. The enqueue checks the lease and fence again in the same atomic step. |
| Fresh, several sessions | The send is rejected. |
| Fresh, no session | One refresh is requested from the destination watcher. |
| Missing or expired | A refresh is requested through Redis. |

After a refresh request the sender waits up to 60 seconds for a newer snapshot. A machine with no live watcher produces a precise error and no enqueue. Local Herdr is a fallback for cold local recovery. The [directory state model](../spec/reviews/relay-directory-state-model.md) states these guarantees and their limits.

## Delivery by lead state

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="images/delivery-dark.svg">
  <img alt="A send looks up the recipient in the directory and queues the message in the recipient's inbox. A working lead gets it from a hook, a starting lead from the SessionStart hook, an idle lead from relay-watcher, and a blocked lead keeps it queued while the operator is notified once." src="images/delivery-light.svg">
</picture>

| Lead state | Delivered by |
|---|---|
| Working | `PostToolUse` hook mid-turn, or `Stop` hook, which blocks the turn from ending while mail is pending |
| Starting | `SessionStart` hook delivers the backlog |
| Idle or done | `relay-watcher` types the batch in with `herdr agent prompt` |
| Blocked on an approval or question dialog | The message stays queued, and the watcher shows one Herdr notification per blocked episode |

The `UserPromptSubmit` hook also delivers.

### Hooks

`agent-msg hook claude` and `agent-msg hook codex` read the harness's hook JSON on stdin and print hook output. A hook always exits 0, so a fault never breaks a lead's turn. A malformed `config.toml` is reported on stderr by the hook, and every other command exits 1.

A hook works inside a time budget:

- A hook that gets no complete stdin payload gives up after about 300 ms of silence, or 900 ms in total while bytes keep arriving.
- When Redis is unreachable, `UserPromptSubmit` gives up within 300 ms. `PostToolUse` and `Stop` give up within 900 ms because they exist to deliver mail.
- A pop is issued only when twice the measured round trip plus 20 ms remains in the budget. Otherwise the hook skips the pop and records why.

A cached hook with an empty inbox costs 2 sequential Redis requests, and one that delivers a batch costs 4. Startup verifies the session through Herdr and registers the queue atomically.

Each failure a hook swallows appends one typed record to `spool/hook.jsonl` beside `config.toml`. A record holds a stage and a stable error code, and never message text. The file is capped at 256 KiB. The next hook that reaches Redis ships the records to the capped `health:hooks` stream by id and removes them from the file. `agent-msg doctor` and the metrics count each id once.

### The watcher

`relay-watcher` runs one process per machine. It blocks on the message log stream, so a send wakes it at once. It then reads the local leads that have mail and asks Herdr for their state:

- Idle or done: it pops the inbox and types the batch with `herdr agent prompt`. It rechecks `state_change_seq` just before typing, so a lead that has just started a turn is left for its own hook.
- Working: it leaves the batch for the hook.
- Blocked: it leaves the batch queued and notifies once.
- No agent in the pane, or a Herdr error: it logs the condition and skips the lead.

A lead left for a hook or a dialog is looked at again every `interval_secs` (default 2 s). A backup sweep every 15 s covers every local lead, so a lost wake costs seconds. The watcher refreshes the directory and the registrations about every 30 s. It retries a failed Redis connection with exponential backoff from 1 s up to 30 s.

## Acknowledgement and redelivery

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="images/lifecycle-dark.svg">
  <img alt="A popped batch moves from the inbox to in flight under a lease of 20 s for a hook or 60 s for the watcher. An acknowledgement ends it. An expired lease returns the batch to the inbox, up to 3 times, and the next expiry moves it to the dead-letter list." src="images/lifecycle-light.svg">
</picture>

A pop moves a batch into the lead's in-flight record in the same Redis script, so a batch is never in two places. The hook acknowledges after its output is written and flushed. The watcher acknowledges when `herdr agent prompt` reports success.

A batch that is not acknowledged returns to the front of the inbox, in order, when its lease expires on the Redis clock. A hook holds a 20-second lease and the watcher a 60-second lease. The hook reaps its own lead's expired batches inside its pop. The watcher reaps every registered lead at start-up and on each sweep.

A crash between delivery and acknowledgement delivers the batch twice. The second delivery is signed `[ix-relay] from <sender> (redelivered): ...`, and the redelivery count travels with the stored entry. After 3 redeliveries the next expiry moves the message to the dead-letter list, which keeps the newest 1,000.

A lead can hold at most 16 unacknowledged batches and 1 MiB of them. A pop that would exceed either takes nothing until a lease frees room. The hook records the refusal, and the watcher logs `in-flight cap reached` once per episode.

Every script checks the type of the keys it writes before its first write, and writes each move's destination before it deletes the source.

The opencode plugin delivers through `agent-msg inbox --json`, which pops without a lease. Its deliveries are at-most-once. A person's `agent-msg inbox` also pops without a lease.

## Stranded mail

A queue belongs to a session and an address names a tab. When a session ends or is replaced, mail sent to the address earlier waits in an inbox that nobody pops.

On each directory refresh the watcher looks at every local registration with queued mail whose pane no longer hosts its session:

- If exactly one other registration holds the same address and Herdr verifies its session in the pane, the queued mail moves to that registration, ahead of its own mail and in order. Each moved message says on its signed first line that it was sent to a prior session of the address, with the original time.
- If no registration holds the address and the stale registration is 24 hours old, the mail moves to the dead-letter list and the registration is pruned.
- If anything is uncertain, such as Herdr being silent, two registrations at the address or a pane that still hosts its session, nothing moves.

Upgrade every machine together after this behavior is added: an older `agent-msg` rejects a forwarded message and does not show it.

## Delivery through the Claude Code inbox socket

With `[watcher] delivery = "socket"`, the watcher posts a batch to the inbox socket that Claude Code binds for the lead's session. The batch arrives as Claude Code's own cross-session message, and no typed text or Stop-hook error label appears. The default is `"herdr"`, and the socket is off until you enable it.

How the socket path behaves:

- The watcher finds the socket by the lead's session ID in `~/.claude/sessions`, which it reads and never writes. It uses a record only when the process is alive with the recorded start time, reports `peerProtocol` 1 and runs Claude Code 2.1.224 or later.
- The watcher binds its own reply socket at `$XDG_RUNTIME_DIR/cc-socks/<pid>.sock` on Linux and `/tmp/cc-socks/<pid>.sock` on macOS.
- Only Claude leads are tried. Codex and opencode leads go straight to Herdr.
- Herdr is the fallback for a lead with no session ID, no live record, a refused connection, an unbound reply socket or a receipt of `refused` or `dropped`.
- A batch is acknowledged when its message ID appears at the end of the lead's transcript (`~/.claude/projects/.../<session>.jsonl`, read only). The watcher waits up to 3 s and posts only when the tick's 8 s socket budget covers the worst case of 7 s.
- Receipts are matched by exact status. `delivered` lets the transcript acknowledge the batch, `denied` dead-letters it, and a hold timeout returns it unmarked.
- While Claude Code holds a message behind an approval prompt, the watcher renews the batch's lease every tick. After 30 minutes without an answer the batch is dead-lettered with the reason `hold_expired` and one notification is shown.
- A decision about a batch is kept until its write to the bus succeeds. The write is retried with a growing wait from 5 s to 5 minutes, and five failures raise one notification.
- Status files are in `~/.ix/relay/state/socket/`, created with mode 0700. Each file is created with mode 0600, and dead-letter reasons in it age out after 24 hours.

A Claude Code session in bypass mode holds every message from another session behind a Deny or Deliver prompt unless `crossSessionInbound: "accept"` is set in its user settings, `--settings` or managed settings. Plugins and project settings cannot set it, and the watcher never writes Claude Code files. For such a lead, the batch stays in flight for ten minutes with no more mail popped, the operator gets one notification per episode and `doctor` warns. Set `crossSessionInbound` to `accept` for that lead, or use `delivery = "herdr"`.

### Wire formats

The socket line, the receipts, `~/.claude/sessions/<pid>.json` and the transcript's `origin.msg_id` are Claude Code formats that are not documented and can change in any release. They are typed in `crates/claude-wire`, which does no I/O. Parsing is total, and each format has a drift signal:

- An unknown reply-socket line, a non-JSON line or an unknown receipt status is counted against the Claude Code version it came from, in hourly buckets that age out after 24 hours. `agent-msg doctor` shows a `socket_wire_drift` warning per version, with the last offending line's `type` and `action` cut to 64 characters.
- A transcript line that names a message ID without a structured `origin.msg_id` acknowledges the batch as received and unconfirmed, and `doctor` shows `socket_transcript_shape_unrecognised`.
- The fixtures in `crates/claude-wire/fixtures/` were captured on Claude Code 2.1.283. Each is marked `captured`, `captured-shape`, `derived-from-program-text` or `synthetic` in its `MANIFEST`. For each version, `doctor` reports whether the socket is working, in drift or only listed.

The inventory of external formats, their owners and their drift signals is in [section 15 of the specification](../spec/spec.md).

## Health reporting

`agent-msg doctor` is read-only, works with Redis down and finishes within 5 seconds. It reports Redis and its round trip, the watcher heartbeat of this machine and of every machine with a lead, the watcher's state file and service unit, hook failures, the Herdr version, registrations, dead letters, in-flight leases and inbox age. Each line has a stable code. A line that is not `ok` names a likely cause and a fix.

| Level | Meaning | Exit |
|---|---|---|
| `ok` | Healthy. | 0 |
| `warn` | Dead letters, a pane that Herdr dropped an hour ago, a watcher older than the heartbeat, an old Herdr with readable output, or mail waiting while the watcher is fresh. | 0, or 1 with `--strict` |
| `fail` | A machine with registered leads whose watcher ticked in the last 24 hours and now has no heartbeat, a fleet-wide burst of 50 or more hook failures in 15 minutes, one session failing repeatedly, or mail waiting under a wedged or missing watcher. | 1 |
| `unknown` | The check could not tell, such as an unreadable service manager. | 0 |

The watcher writes its heartbeat to `<prefix>health:watcher:<machine>` after every good tick, with an expiry of three sweep intervals and at least 30 seconds. A missing key means no watcher has ticked on that machine recently.

## Lane reports and the work dashboard

Leads can publish what they are working on. Every value is reported explicitly, and nothing is inferred from tickets or activity.

| Command | Effect |
|---|---|
| `agent-msg focus "<text>"` and `agent-msg focus --clear` | Set or clear the calling lead's focus. |
| `agent-msg who --focus [--stale-after 12h]` | Show every lead's reported focus, its set time or an unset state. A focus older than the threshold (default `7d`) is marked stale and stays visible. The threshold is an integer with one suffix: `s`, `m`, `h` or `d`. |
| `agent-msg server-capacity set <server> --cores N --ram-gib N [--max-agents N] [--owner-rule <text>]` | Record operator-entered capacity for a server. RAM is stored in MiB, and `--owner-rule` can repeat. |
| `agent-msg server-capacity clear <server>` | Remove that server's capacity record. |
| `agent-msg who --servers` | Group leads by server with the recorded capacity, or an unset state. |
| `agent-msg status-request all` or `<address>,<address>` | Ask selected leads for the fixed program-review status form. Prints a request ID. |
| `agent-msg status-reply <id> --state active\|idle\|blocked --spare-coder-slots N` | A lead's structured reply. `--ticket`, `--pull-request`, `--blocker-ticket` and `--owner-decision` can repeat. |
| `agent-msg status-replies <id>` | JSON of the replies and the leads that have not replied. |

`agent-msg work` reports a lead's scope, focus and agent assignments for `team-leader-dashboard`:

```bash
agent-msg work scope --project <project> --initiative <initiative>
agent-msg work focus "Reviewing credentials" --ticket <ticket>
agent-msg work agent <id> --ticket <ticket> --role reviewer --status running
```

`team-leader-dashboard --lead <address>` monitors one lead session and never consumes inboxes or changes tracker state. It reads Herdr, Linear (with `LINEAR_API_KEY`) and the lead's work reports. `--session <native-session-id>` refuses an unexpected occupant at start-up. The monitor keeps the original session and process binding, so restart it to follow a replacement.

The dashboard has three views: `t` for tickets, `a` for agents and `p` for scope. Scroll with the arrows or `j` and `k`, open details with `d` or Enter, force a Linear refresh with `r`, show source timestamps with `s` and quit with `q`. Linear refreshes every 30 minutes, and Herdr and the reports refresh every minute. Native internal subagents appear only when the lead reports them. Scope counts are limited to the reported scope, and overlapping initiative and project counts must not be added.
