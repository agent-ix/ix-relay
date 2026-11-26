---
id: FR-044
title: "The watcher delivers to Claude Code sessions through their inbox socket"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-002"
    type: "implements"
---
# FR-044: The watcher delivers to Claude Code sessions through their inbox socket

## Description

`relay-watcher` shall post a popped batch to the inbox socket Claude Code binds for
the lead's session, so the message arrives as Claude Code's own cross-session message
instead of typed text. This is the default: it needs no setting.

**A Claude Code session that has an inbox socket receives relay mail only through that
socket.** Whether it is working, idle or blocked, and whatever happens to a post
(certain non-delivery, an unknown outcome, an approval hold, a duplicate body, a
watcher restart, a redelivery, mail forwarded from a prior session of the address),
the watcher shall never type that mail into its terminal and its hooks shall not
consume it. Mail that is not delivered stays queued or in flight, and is retried or
dead-lettered with a reason `doctor` or the watcher's log shows.

herdr types mail only into a lead that has no socket at all: another harness whose
session no record names ([FR-054](./FR-054-socket-delivery-is-for-claude-leads.md)), or a
Claude Code session whose own registry record shows it has none (below).

`[watcher] delivery = "herdr"` is the one exception, and only an operator sets it: it
switches inbox sockets off for the whole machine, so every batch, for Claude Code
sessions with a socket too, is typed through herdr and their hooks deliver mail.

**The wire format is not documented by Claude Code.** It was measured on Claude
Code 2.1.283 (AGE-2030) and can change in any release. That is why a session is
used only when its registry record reports a known `peerProtocol` and version.
A session whose socket cannot be used waits; it is not typed into. The formats are
typed in the `claude-wire` crate, and what does not parse is counted as drift, not
dropped
([FR-046](./FR-046-claude-code-formats-are-typed-and-drift-is-detected.md)).

A receiver in bypass mode holds every peer message behind an approval prompt unless
`crossSessionInbound: "accept"` is set in its user settings, `--settings` or managed
settings (project settings and plugins cannot set it). Whether to set it is the
operator's decision; this requirement only makes the watcher notice.

## Behavior

### Which transport carries a lead's mail

- Where `[watcher] delivery` is absent or `"socket"`, the watcher shall use the inbox
  socket of every Claude Code session that has one.
- Where `[watcher] delivery` is `"herdr"`, the watcher shall type every batch through
  herdr, shall not read Claude Code's registry, bind a socket or write a status file,
  and shall say at start that sockets are switched off. An unknown value is a
  configuration error ([FR-027](./FR-027-malformed-configuration-per-command.md)).
- Before popping anything for a lead the watcher shall decide, from the
  registry, one of three routes for its registered session: the socket, the terminal,
  or waiting.
- The watcher shall choose the socket when the session resolves to a usable socket and
  the reply socket is bound.
- The watcher shall choose the terminal for a `claude` lead only when no live record of
  the session is parked, every live record names no `messagingSocketPath`, no
  `<pid>.sock` of those processes exists in a socket directory, and each process
  started at least 60 s ago (Claude Code writes the record before it publishes the
  socket into it).
- In every other case the watcher shall make the lead wait: nothing is popped, nothing
  is typed, the reason is logged once per stretch and kept in the status file. These
  cases are: no session id, no record, no live record, several live processes, an
  unknown protocol or version, a socket path that is unusable, a record that is not
  old enough to prove it has no socket, a parked session, an unbound reply socket, an
  unreadable registry and socket locations that cannot be told on this host.
- When mail for a lead has waited 5 minutes because its socket could not take it (a wait,
  a certain non-delivery, a deferral or a batch put back by a later decision), the watcher
  shall show one notification naming the lead and the newest reason, and shall type
  nothing. The stretch ends when mail gets through or none is left, queued or posted. It is
  paused while a socket holds the mail unconsumed, and keeps its start if the mail comes
  back. A notification that cannot be shown is tried again after a minute.
- If sockets cannot be used on the host, then the watcher shall write a status file that
  says why, remove it when it exits, and `doctor` shall warn with that reason.
- A record with `parkedJobId` belongs to a process that handed its terminal to a
  background job: it still listens and queues what is posted but consumes none of it,
  and its pane shows another conversation. The watcher shall skip such a record, and
  shall make a session with no other live record wait.
- The watcher shall not read herdr's pane state before posting to a socket, and shall
  not gate a post on pane activity.
- The watcher shall type through herdr only after the checks of
  [FR-014](./FR-014-watcher-types-only-into-the-registered-session.md), and shall ask
  for the route again immediately before typing; if the lead is no longer on the
  terminal, the batch goes back unmarked and nothing is typed.
- Where sockets are used, a Claude Code hook shall register the lead and record its
  activity without popping or acknowledging inbox messages when its session has a
  socket (`CLAUDE_CODE_MESSAGING_SOCKET` in its environment, or a registry record of
  the session that names one). A hook of a session with no socket delivers as before.
  While the watcher is stopped, a socket session's mail stays queued.
- If the socket certainly did not take a batch, then the watcher shall put it back and
  post it again on a later tick. It shall not type it.

### The socket

- For each batch, the watcher shall look up the target by the lead's registered
  session id in Claude Code's session registry (`~/.claude/sessions/*.json`, read
  only), every time, because `/clear` changes the session id of a record and a
  resumed session is a new process with a new socket.
- The watcher shall use a record only when its process is the one that wrote it:
  the pid is alive, its start time equals the record's `procStart`, and the record's
  `pidDomain` equals this machine's. Among the survivors it shall prefer
  `interactive` processes, and shall refuse when more than one remains.
- The watcher shall use a session only when its record gives `peerProtocol` 1 and a
  version of 2.1.224 or later, and a socket that is that process's own `<pid>.sock`,
  in the socket directory (`$XDG_RUNTIME_DIR/cc-socks`, or `/tmp/cc-socks-<uid>`),
  not a symlink, owned by this user and not open to others. It does not compare the
  connecting peer's pid with the record's (`SO_PEERCRED`): that needs `unsafe` or a
  new dependency.
- The watcher shall post one line of newline-delimited JSON, on a new connection per
  batch: `msgV` 1, a random UUID `msg_id` (Claude Code records the id in its
  transcript only when it is a UUID), `type` `user`, the target `session_id`,
  `priority` `next`, `from` `uds:<the watcher's reply socket>` and a `message` of
  role `user` whose content is the rendered batch
  ([FR-016](./FR-016-render-neutralises-control-characters.md)) inside
  `<cross-session-message from="uds:…" from-name="<sender addresses>">`.
- The watcher shall write every `<` in the text of a batch as `&lt;` (it is display
  text), so that no message can close the wrapper or open a second one, whatever the
  case, spacing or invisible character it puts between the `<` and the tag; the
  attributes escape `&`, `"`, `<` and `>`.
- The watcher shall not assert `from-mode`.
- The watcher shall write `from-name` as one plain line of at most 200 characters.
- The watcher shall bind its reply socket at `$XDG_RUNTIME_DIR/cc-socks/<watcher pid>.sock`
  with mode 0600 and keep listening on it for the life of the watcher, so a receipt
  that arrives minutes after a delivery is heard. It shall stop at the next turn of its
  loop on SIGTERM or SIGINT (`signal_hook::flag`, already a workspace dependency, safe,
  as `relay-tui` uses it) and remove the reply socket and its status file on the way
  out, keeping the status file only when it holds a held batch for its successor. A
  watcher killed by SIGKILL leaves them, and the next start removes only what a
  previous, now dead, watcher recorded in its status file. A watcher is dead when its
  pid has no process, or the process there started at another time than the status
  records (a reused pid).
- If the reply socket cannot be bound (the directory may not exist at login), then the
  watcher shall post nothing and type nothing for a socket session, say so once per episode,
  show `reply socket not bound` in `doctor`, and try again with a growing wait. It shall not
  remove a `<pid>.sock` that a live listener answers on, or that is not a socket.
- While Claude Code holds a message, the watcher shall extend the lease of that batch
  on each tick (`Relay::renew_batch`, which touches only the named batch, never shortens a
  lease and matches the lead and the popper), so that neither the reaper, the lead's own
  hook nor the watcher pops or re-posts it while a copy may still be held behind the prompt.
- While the receiving process remains live and the batch is held for approval, the watcher
  shall keep the batch in flight and renew its lease each tick without a time limit. It shall
  stop renewing and settle the batch only when the receiver settles it or the receiving
  process is verified gone. An unanswered approval prompt shall not be dead-lettered due to
  age.
- When a `delivered` receipt arrives, the watcher shall acknowledge the batch once the
  transcript shows the message.
- When a `denied` receipt arrives, the watcher shall dead-letter the batch exactly once, without
  re-prompting.
- The watcher shall keep a decision about a batch (acknowledge, put back, dead-letter) pending
  until the write to the bus has succeeded.
- While a decision's write fails, the watcher shall renew the batch's lease and retry the write
  after a wait that doubles from 5 s to at most 5 minutes.
- When five writes of one decision have failed, the watcher shall show one notification and
  record a `settlement_failed` note for the lead in its status file.
- If a decision's write keeps failing, then the watcher shall keep the batch in flight and post nothing
  more for its message.
- When a decision's write succeeds, the watcher shall clear the `settlement_failed` note.
- If a connection error ends a tick, then the watcher shall keep the decisions still owed for
  the next tick.
- When Claude Code has reported a held message delivered and its transcript line is not found
  within 5 minutes, the watcher shall acknowledge the batch and record a
  `delivered_unconfirmed` note.
- The watcher shall not acknowledge a batch on a `delivered` receipt alone before that grace ends.
- The watcher shall not dead-letter an approved message as never approved.
- The watcher shall keep the held batches, and when each was reported delivered, in its status
  file, so that a restarted watcher adopts them, goes on renewing them and continues the grace.
- The watcher shall keep the decisions still owed a write to the bus in its status file, and a
  restarted watcher shall apply them after adopting, validated like the held batches, so that an
  owed acknowledgement is not lost and an owed denial is not turned into `hold_expired`.
- The watcher shall keep a `settlement_failed` note apart from the lead's other note, so that a
  later delivery note does not overwrite it, and shall clear it only when that batch's decision
  is written; the note is not saved in the status file, so after a restart it is raised again
  after five further failures.
- If a batch's renewal is missed for more than 120 s (the watcher or the bus down or unreachable)
  and its lease lapses, then the watcher shall keep the lead marked held, popping nothing for it,
  until the quiet period ends, and `doctor` shall show the lead as held.
- When a restarted watcher finds no status file, the watcher shall treat the lead as not held.
- When a restarted watcher finds no status file, the watcher shall redeliver the batch, marked `(redelivered)`, after its lease expires.
- Known limit of a lost status file: the watcher posts the redelivered batch again while the socket copy is still held, so an approval duplicates it. (The hook of a session with a socket pops nothing.)
- The watcher shall adopt a restored batch only when its batch id is not 0, its lead and ids are
  valid names, and its times compute (checked arithmetic) to a moment neither in the future nor
  more than 30 days ago.
- The watcher shall apply the caps of the status file (256 open sends, 32 dead-letter reasons)
  to what it adopts, and drop reasons older than 24 hours.
- The bus shall renew, dead-letter, acknowledge or requeue a batch only when its id, its lead
  and its popper match the caller's (the watcher's calls name the watcher, the hook's name the
  hook), so an id that the other consumer holds is left alone. A Redis counter that
  was reset can make a stale id collide with another batch the watcher itself popped: that is
  not detectable.
- The watcher shall keep, per lead, the reason (`denied`, `hold_expired`) and batch id of every
  batch it dead-lettered on its own account in the status file for 24 hours, where a later note
  does not overwrite it, and `doctor` shall show them. A dead-letter entry is a bare message
  with no reason and no batch id, so `agent-msg inbox --dead` cannot show which entry it was: the
  entry format is not changed.
- The watcher shall keep its status files in a directory of their own (`state/socket`), create it
  with mode 0700 in one step, create each file with mode 0600 and write it to disk before the
  rename.
- The watcher shall use an existing status directory only when it is a real directory (not a
  symlink) owned by the current user with mode 0700, and shall neither repair nor read any other
  (a file planted while a directory was open must never become trusted by a later chmod).
- `doctor` shall report a status directory the watcher does not trust as unreadable.
- The watcher and `doctor` shall not read a status file that is a symlink or is owned by another
  user.
- The watcher shall acknowledge a batch only when its `msg_id` appears as `origin.msg_id` in
  the target's transcript (`~/.claude/projects/<project>/<session id>.jsonl`, read only)
  ([FR-020](./FR-020-hook-never-loses-popped-messages.md)).
- The watcher shall take the transcript's length before it writes the line to the socket, so
  that a receiver that records the message at once is never missed.
- The watcher shall wait 3 s after posting for a receipt or the transcript line.
- The watcher shall read the transcript in windows of at most 512 KiB from where the message was
  posted, and check at most 32 open sends' transcripts per tick, in turn, so that a transcript
  that grows by more than a window between looks still yields the acknowledgement.
- The watcher shall post a batch only when the tick's remaining socket budget covers 7 s: a 2 s
  connect, a 2 s write and the 3 s wait. The budget is 8 s per tick.
- The watcher shall count against the tick's budget the time it spends resolving targets,
  connecting, writing, waiting and checking transcripts in that tick.
- If the budget does not cover a post, then the watcher shall put the batch back unposted and
  unmarked, with no notification and no episode, and shall try it on the next tick.
- If 256 posts are already open, then the watcher shall post nothing more and put the batch
  back unmarked until one is settled.
- Each tick the watcher shall look at the leads in a rotating order, so that a lead with mail is
  reached within as many ticks as there are leads.
- If the ack wait ends without a transcript record, the posted batch shall remain
  pending with its lease renewed. Delayed consumption shall not stop later batches from
  being posted. Pending consumption shall survive watcher restart without becoming an
  approval hold. A partially written batch shall retain the same ownership.
- When a posted batch has not been consumed for 30 minutes and its receiver is alive (an
  open dialog or a long tool call), the watcher shall keep it in flight, shall not post it
  again, dead-letter or type it, shall note it in the status file and shall show one
  notification.
- At most once in 10 s for each pending batch that Claude Code has not reported delivered,
  the watcher shall look at the process it posted to. If that process is gone (no process
  with the pid, or one that started at another time; a look that fails proves nothing),
  then the watcher shall read the transcript to its end, acknowledge the batch if it shows
  the message, and otherwise put the batch back to be delivered again.
- The watcher shall keep with each pending batch in its status file the process it was
  posted to and where the transcript ended before the post, so that a restarted watcher
  adopts the batch at any age up to 30 days, reads the transcript from that point, and
  neither posts a consumed message again nor misses one followed by more than a read
  window.
- If a transcript line names a message's id without a structured `origin.msg_id`, then the batch is
  not ambiguous: it is acknowledged as received but unconfirmed, and
  nothing is redelivered ([FR-046](./FR-046-claude-code-formats-are-typed-and-drift-is-detected.md)).
- When a batch is ambiguous, or posted for 5 minutes without its transcript showing it, or
  its transcript cannot be found, `doctor` shall warn and name the possible causes (a
  transcript under another `CLAUDE_CONFIG_DIR`, a socket that wants authentication, a
  cleared or busy session, a changed wire format).
- The watcher shall act on a receipt (`type` `control`, `action` `peer_message_status`)
  by its exact `status` and `status_detail`, never by substring.
- When a `held` receipt arrives, the watcher shall leave the batch in flight, raise one
  notification per episode naming the lead and the way out (set `crossSessionInbound` to
  accept in that session's settings), and pop nothing for that lead, and post nothing to it,
  for ten minutes, so mail sent meanwhile stays in the inbox instead of expiring in flight.
- When a `refused` receipt (or `expired` with detail `refused`) or a `dropped` receipt (which
  names its messages in `dropped_msg_ids`) arrives, the watcher shall put the batch back
  unmarked, to be posted again, and shall not type it.
- When an `expired` receipt with detail `hold_timeout` arrives, the watcher shall put the batch
  back unmarked.
- When a receipt has any other status, or reports a duplicate, the watcher shall treat the batch
  as ambiguous, leave a note and keep it with the socket as it keeps an unconfirmed post.
- Mail can be delivered out of order after the quiet period: the watcher can post again
  behind the same prompt while the held batch stays renewed, so a later approval can
  deliver older mail after newer. A socket session's hooks do not consume newer mail.
- The watcher shall not post the same body to the same session twice within 35 s (Claude Code
  drops an identical body from one sender within 30 s). A redelivered batch carries the
  `(redelivered)` marker, so its body differs.
- The watcher shall give up a connect after 2 s.
- The watcher shall give up the write of one line after 2 s in total (one deadline for the whole
  line, however slowly the receiver reads), and retain an uncertain post in flight.
- A session that stopped accepting shall cost one blocked helper thread and no more.
- When the transport postpones a batch, the watcher shall leave the lead's episode state and its
  record of who was notified unchanged: no episode ends, nothing is cleared, and nothing is logged.
- If the status directory is not trusted or cannot be written, then the watcher shall keep
  delivering, log it once per episode, show one notification, and `doctor` shall say plainly
  `socket status directory untrusted: held batches will not survive a restart`; the decisions
  still owed a write are then lost on a restart, and the watcher says so instead of losing them
  silently.
- The watcher shall never write to a Claude Code session, registry or settings file and never
  connect to a socket that is not a resolved, live session's.
- The watcher shall use the socket of a session whatever kind its lead registered
  ([FR-054](./FR-054-socket-delivery-is-for-claude-leads.md)), and shall keep the resolved
  leads of its status file to registered leads with a live process
  ([FR-055](./FR-055-socket-status-keeps-only-live-leads.md)).
- The watcher shall write `state/socket/socket-<watcher pid>.json` (its pid and start time, its
  reply socket, the sockets it resolved, the leads held, the held batches, the dead-letter reasons
  and why a lead's mail waits) after each socket delivery or change of reason, so two watchers on one
  machine do not overwrite each other.
- `agent-msg doctor` shall report each live watcher's file as `ok`, `warn` or `unknown`, never as
  `fail`, and `unknown` when `config.toml` did not load.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-044-AC-1 | Given `[watcher] delivery` absent or `"socket"`, the mode is socket; given `"herdr"` it is herdr and every batch passes to herdr; given any other value the configuration is refused by the watcher only. | Test |
| FR-044-AC-2 | Given a registry with a live record, a dead pid, a reused pid, another or unknown pid domain, two live processes on one session, a session whose id `/clear` changed, an unknown `peerProtocol` or version, and a socket that is a symlink, in another directory, of another owner, open to others or not the process's own, only the live, known-good, single (interactive-preferred) record with a proper socket resolves. | Test |
| FR-044-AC-3 | The line posted is exactly the measured one (golden), carries a UUID `msg_id`, the target session and the reply socket (mode 0600), asserts no `from-mode`, cannot be made to close or forge its wrapper, has a plain bounded `from-name`, and is not posted over 1 MiB; the exact receipt statuses parse to their outcomes (only `held` and `refused` were captured live from Claude Code 2.1.283; the others were read from the installed program's text and are fixtures only), a status that only contains a known word does not, and junk parses to none. | Test |
| FR-044-AC-4 | A batch is acknowledged only when its `msg_id` is the `origin.msg_id` of a transcript line; an absent transcript or one without the id leaves it ambiguous, and the id in a line without a structured `origin.msg_id` is acknowledged as received but unconfirmed (FR-046-AC-5); a receiver that records the message at once into an existing transcript, with 0, 20 or 200 project directories to search, is never missed in 200 rounds; a transcript that grows past a read window between looks still yields the acknowledgement; a message that outlived its wait is acknowledged by a later tick. | Test |
| FR-044-AC-5 | A `held` receipt leaves the batch in flight, pops nothing more for the lead, notifies once and dead-letters no mail sent meanwhile across lease expiries; the held batch's lease is renewed each tick so a reap past its old deadline and the lead's own hook find it in flight; an approval after the lease would have run out acknowledges once with one post in all; a denial after it dead-letters exactly once and never re-prompts; a prompt unanswered for more than 30 minutes remains in flight without another post or age-based dead-letter; the batch is settled only after receiver settlement or verified receiver process exit; a batch held across a restart is restored and still renewed. | Test |
| FR-044-AC-6 | In socket mode a batch for a session with a socket is never typed: after certain non-delivery (no record, a refused connection, an unbound reply socket, a `refused` or `dropped` receipt, a duplicate body) it is queued for a later socket attempt, and after an unknown or unconfirmed outcome it stays in flight with the socket. A lead of another harness whose session no record names is typed through herdr. | Test |
| FR-044-AC-7 | The same body is not posted to a session twice inside the window and is once the window has passed; a redelivered body differs from the original. | Test |
| FR-044-AC-8 | A delivery changes no file of the registry or the projects; a live listener on the watcher's socket name, a non-socket file and a dead socket there are left, left and replaced; a previous watcher's dead socket and status file are removed (a reused pid counts as dead), and a live watcher's, another process's name or a path outside the socket directory are not; the reply socket and status file are removed on SIGTERM (the file kept only while a held batch or an owed decision must survive), and the interrupted wait of that stop is not recorded as a failure. | Test |
| FR-044-AC-9 | Each live watcher's status has its own doctor check that is `ok`, `warn` (held, leads waiting for a socket, a reply socket not bound, dead letters made) or `unknown` (missing, unreadable, dead watcher, config failed), never `fail`, a held lead is named with the setting that clears it, a batch dead-lettered as `denied` or `hold_expired` is shown with its reason for 24 hours, a young unconsumed post is not a problem while one 5 minutes old or with no transcript warns with the possible causes, sockets that cannot be used are a warning with the reason, `delivery = "herdr"` is a warning, and an untrusted status directory is named plainly. | Test |
| FR-044-AC-10 | A late drop names its messages and puts exactly those back, a hold timeout or late refusal puts the batch back unmarked, an unknown or ambiguous status settles nothing and leaves a note, and a receipt for a message nothing waits on changes nothing. | Test |
| FR-044-AC-11 | A failed dead-letter, acknowledge or in-wait denial write is retried after a growing wait and never becomes a second post; after five failures one alert is raised, the decision stays pending and the batch is renewed; a dead-letter write that fails for twenty ticks leaves the message posted once; the `settlement_failed` note survives a later note for the lead and is cleared only by its own batch's write; owed decisions survive a restart (an owed denial stays `denied`, an owed acknowledgement is not lost). | Test |
| FR-044-AC-12 | An approved message whose transcript line never shows is acknowledged after the grace with a `delivered_unconfirmed` note, never dead-lettered as `hold_expired`, also when the watcher restarted inside the grace; a `delivered` receipt alone does not acknowledge before the grace ends. | Test |
| FR-044-AC-13 | A restored batch is adopted only when every field validates (forged epoch-zero, future, oversized and `i64` edge times, batch id 0, bad names), the caps apply, a batch id that the lead's own hook holds is neither renewed, dead-lettered, acknowledged nor requeued (also as an owed decision restored from a status file), owed decisions are validated the same way, and dead-letter reasons older than 24 hours are dropped on adoption. | Test |
| FR-044-AC-14 | A batch that does not fit the tick's remaining budget (the worst case of a post) is put back unposted with no notification and no episode, so a following bypass-mode hold is still notified once; at the production timings a healthy lead after a wedged one is postponed and then delivered, a healthy lead behind twenty wedged ones is served within one round of ticks, a postponement leaves a held episode and its notification record untouched (held, postponed, held gives one notification and no "ended" line), and a write to a receiver that reads slowly gives up at the 2 s deadline as unconfirmed. | Test |
| FR-044-AC-15 | The status files are in a directory of their own created 0700 in one step; an existing directory that is open (0755, 0777), a symlink or of another user is neither written, read nor repaired (a forged file planted while it was open is never adopted, before or after a later start); a symlinked or foreign status file is not read; files are created 0600; when the directory is not trusted delivery goes on and one alert says held batches and owed decisions will not survive a restart. | Test |
| FR-044-AC-16 | A reply socket whose directory appears later is bound on a later delivery, not more often than its backoff; a connect that never returns is abandoned and costs one helper thread in all; a probe of a wedged listener at the watcher's own socket name keeps one flag across retries. | Test |
| FR-044-AC-17 | Working, blocked, idle and done Claude leads receive queued mail through a real Unix socket without a herdr state read. Whatever the post answers, nothing is typed. A Claude hook of a session with a socket registers the lead for all four events without popping or acknowledging mail; a hook of a session without one delivers it. | Test |
| FR-044-AC-18 | Pending socket consumption persists across watcher restart, renews its lease without an approval hold and acknowledges a later transcript match once; past the maximum hold with a live receiver it is neither dead-lettered nor posted again, the status file notes it and one notification is shown. | Test |
| FR-044-AC-19 | The route is the terminal only for a lead of another harness whose session no record names, for `delivery = "herdr"`, or for a Claude session that has no parked record and whose live records name no socket, with no socket file for their processes, each older than the publish grace. It is a wait, with nothing popped or typed, for no session id, no or a stale record, several processes, an unsupported or unusable record, a parked record without a live socket record, a record younger than the grace, a socket file the record does not name, an unbound reply socket and unavailable socket locations. Typing asks the route again and types nothing when it changed. | Test |
| FR-044-AC-20 | A session whose only live record is parked, listening on its socket and consuming nothing, in a pane herdr reports idle with that session, is neither posted to nor typed into, its forwarded and redelivered mail stays queued with the reason shown, and mail is posted once the record is no longer parked. | Test |
| FR-044-AC-21 | Mail that has waited past the bound for a socket raises one notification with the newest reason and nothing typed; delivery ends the stretch and a later wait is announced again; a notification that failed is retried. | Test |
| FR-044-AC-22 | A pending batch whose receiving process exited or whose pid was reused is put back when the transcript does not show it and acknowledged when it does; a session resumed in a new process receives it on the new socket and nothing is dead-lettered. | Test |
| FR-044-AC-23 | A watcher restarted two hours after a post adopts it, acknowledges one the transcript shows with more than a read window written after it, keeps one it does not show, and posts neither again. | Test |
| FR-044-AC-24 | With 256 posts open, a further batch is put back unposted and no open post is dropped from the status file. | Test |

## Dependencies

- **Upstream**: [US-002](../usecase/US-002-receive-messages-without-losing-the-turn.md)
- Related: [FR-020](./FR-020-hook-never-loses-popped-messages.md) (the in-flight and
  acknowledgement contract this transport keeps),
  [FR-038](./FR-038-watcher-wakes-when-a-message-is-sent.md) (what wakes the watcher;
  the decision to deliver is unchanged),
  [FR-040](./FR-040-redelivery-is-marked-bounded-and-dead-lettered.md) (redelivery and
  dead-letter behavior for receiver disappearance), [FR-029](./FR-029-doctor-reports-relay-health.md).
- Related: [FR-046](./FR-046-claude-code-formats-are-typed-and-drift-is-detected.md) (the
  typed formats and their drift signals).
- Delivered by AGE-2030; made the default by SWM-121, SWM-122, SWM-123 and SWM-124.
