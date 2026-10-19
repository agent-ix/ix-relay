---
id: FR-029
title: "`agent-msg doctor` reports relay health"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-006"
    type: "implements"
---
# FR-029: `agent-msg doctor` reports relay health

## Description

When the operator runs `agent-msg doctor`, `agent-msg` shall report, for every
machine that has a registered lead, whether a watcher heartbeat is live, and for
the local machine whether Redis is reachable, whether herdr passes its version and
shape check, the hook failures recorded in the last 24 hours and the dead letters,
and shall exit 0 unless a check failed.

## Behavior

- The doctor command shall read no inbox and pop nothing.
- The doctor command shall write nothing: no registration, no spool record, no
  state file and no key on the bus, and shall renew no expiry.
- The doctor command shall run only commands that read (`herdr --version`,
  `herdr agent list`, `herdr agent get`, `systemctl --user is-active` and `cat`,
  `launchctl list`).
- The doctor command shall name each check that did not pass on its own line with
  the machine or lead it concerns, and shall give each a stable code, a message,
  a likely cause and a suggested fix.
- The doctor command shall give each check one of four severities: `ok`; `fail`,
  which makes the exit status 1; `warn`, which is reported and exits 0 unless
  `--strict` is given; and `unknown`, which is reported and never fails the exit
  status. With `--json` it shall print one object `{"ok", "strict", "checks"}`
  whose checks have the members `check`, `code`, `status`, `subject`, `message`,
  `cause`, `fix` and `details`. The source pins the code and member lists.
- If Redis cannot be reached, then the doctor command shall report that check as
  failed, report the local checks that do not need Redis, and exit 1.
- If Redis cannot be reached, then the doctor command shall give the check the
  cause and fix of what the connect hit: a refused password, a refused
  connection, no answer, or no route to the bus (the transport is WireGuard to
  relay-redis on hub, README.md "Transport"; AGE-2067).
- The doctor command shall print the Redis URL only with its credentials
  redacted.
- The doctor command shall finish within 5 s however Redis, herdr and the service
  manager are failing: it shall give the Redis section a budget of 2 s shared by
  the connect and every read, including every `SCAN` call of a key scan (a scan
  that runs out of it stops, keeps what it found and says it was cut by the
  deadline), run the child processes after it in parallel each killed at its own
  deadline together with what they started (their process group), and report a
  read that finds the budget spent as such.
- If the doctor command's output pipe is closed, then it shall exit with the
  status the relay's health gives, not 1.
- The doctor command shall show the members of each live heartbeat (its uptime,
  its last sweep, a Redis flap ridden out since the previous heartbeat, and the
  last failed tick).
- The doctor command shall find in-flight and dead-letter lists with the same
  bounded key scan as `agent-msg inbox --in-flight` (at most 1,000 keys and 64
  `SCAN` calls, the prefix glob-escaped) and shall report a cut scan as
  `backlog_scan_truncated`; it shall read this machine's heartbeat and every
  machine with a registered lead by name, and scan for the rest.
- The doctor command shall give `hook_failures_recorded` the severity `fail` only when, among
  the failures recorded in the last 24 hours, either (a) at least 50 are under 15 minutes old,
  fleet-wide, or (b) one session has failed in each of at least three distinct 15-minute
  windows of the last hour and its newest failure is under 15 minutes old. Otherwise it shall
  give the code `warn`. A single recent record, or a short burst on one machine, is a `warn`.
  The thresholds are the doctor's own: 50 in 15 minutes is more than 15 times the fleet's
  observed average of about 3 per 15 minutes (about 300 a day), so only an outage reaches it;
  a connect-timeout burst ends within one window, so failures in three windows mean the
  failure persisted for over half an hour and is not a burst. The session in (b) is identified
  by its machine and session id (records with no session id are grouped by machine and lead).
- The `hook_failures_recorded` detail shall name, whatever the severity, the three sessions
  with the most failures in the 24 hours (machine, lead, session id as the record cut it, the
  count, the stage and error kind that repeat most, and the newest time) and the three machines
  with the most failures, and shall say which rule made it a `fail`.
- The doctor command shall show text from the bus, a binary or a file, and message
  text of a failed requeue (the only copy), as one neutralised line free of
  control characters.

### Catalogue

| Check | Code | Severity |
|---|---|---|
| config | `config_invalid` | fail |
| | `watcher_otlp_config_invalid` (a watcher-only `otlp_endpoint`, `otlp_headers` or `otlp_interval_secs` in the file that the watcher would refuse; [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md)) | warn |
| Redis | `redis_unreachable` | fail |
| | `redis_slow` (over 1 s) | warn |
| | `redis_read_failed` (also: budget spent) | fail |
| watcher heartbeat, this machine and every machine with a lead seen in 24 h or a last-tick marker | `watcher_heartbeat_missing`: "no watcher heartbeat from X (watcher stopped, or older than the heartbeat)" | fail while the machine has a registered lead and its watcher's non-expiring last-tick marker is under 24 h old (the prune horizon), or a lead seen in the last hour (a watcher older than the marker); warn after that, and warn when a local watcher visibly runs (live pid, fresh state file) without publishing one |
| | `watcher_heartbeat_stale` (last tick over two sweeps and 10 s old, before the key expires) | fail |
| | `watcher_heartbeat_unreadable` | warn |
| | `watcher_herdr_problem` | fail when the watcher types into no pane, else warn |
| | `watcher_otlp_failing` (the watcher cannot push metrics to its collector and is backing off; deliveries are not affected; [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md)) | warn |
| | `heartbeat_scan_truncated` (says which bound: keys, calls, or the time budget) | warn |
| `state/watcher.json` against the heartbeat | `watcher_pid_gone`, `watcher_file_stale` | fail |
| | `watcher_file_missing` | fail without a live heartbeat, else warn |
| | `watcher_file_unreadable`, `watcher_file_mismatch` | warn |
| | `watcher_file_unlocated` | unknown |
| hook spool | `hook_spool_pending` | fail while the newest record (the file's last write, when none decodes) is under 15 min old, else warn |
| | `hook_spool_unreadable` | warn |
| | `hook_spool_unlocated` | unknown |
| hook failures on the bus, 24 h, by stage and error kind | `hook_failures_recorded` | fail while the recent failures reach the volume threshold or a session keeps failing (below), else warn |
| herdr | `herdr_missing`, `herdr_shape_mismatch` | fail |
| | `herdr_unsupported_version`, `herdr_version_unreadable` | warn |
| | `herdr_unchecked` | unknown |
| registrations | `registration_stale` (over 24 h), `registration_no_place`, `registration_undecodable` | warn |
| | `registration_pane_gone` (only once the stamp is an hour old) | warn |
| | `panes_unchecked` | unknown |
| dead letters | `dead_letters_present`, with the hint `agent-msg inbox --dead` | warn |
| in-flight, in entries and batches | `inflight_overdue` (a lease over 2 min past its deadline) | warn |
| | `backlog_scan_truncated` | warn |
| inbox depth and oldest age | `inbox_stuck` (a message over 5 min old) | warn while the lead's machine's heartbeat is fresh (the lead may be busy in a long call); fail when that watcher is wedged, or missing where `watcher_heartbeat_missing` would fail |
| systemd or launchd unit | `service_inactive` | fail without a live local heartbeat, else warn |
| | `service_not_installed` | warn |
| | `service_unknown` | unknown |

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-029-AC-1 | Given every machine with a registered lead has a live heartbeat, herdr passes its check and there are no hook failures or dead letters, `agent-msg doctor` exits 0, even with `--strict`. | Test |
| FR-029-AC-2 | Given `node-a` has a lead seen a minute ago and no live heartbeat, `agent-msg doctor` exits 1 and prints a line naming `node-a` and the missing watcher; given its leads were last seen five hours ago the line is a warning and the exit is 0. | Test |
| FR-029-AC-3 | Given Redis is unreachable, `agent-msg doctor` exits 1 within 5 s and still reports the herdr check; given a Redis that accepts and never answers and a `herdr` and a `systemctl` that hang, it exits 1 within 5 s and reports the two as unknown. | Test |
| FR-029-AC-4 | Given a Redis URL containing a password, no output line of `agent-msg doctor` contains that password. | Test |
| FR-029-AC-5 | Given leads with queued messages, dead letters, in-flight rows and a heartbeat with an expiry, the bytes of every key and whether it has an expiry are the same before and after `agent-msg doctor`, and no local file is created. | Test |
| FR-029-AC-6 | `agent-msg doctor --json` prints one JSON object with exactly the members `ok`, `strict` and `checks`, each check with exactly the documented members and a code from the pinned list. | Test |
| FR-029-AC-7 | Each code fires on the facts that define it with the severity in the catalogue, carries a cause and a fix, and fails the exit status only as a `fail` (or, with `--strict`, a `warn`); an `unknown` never does. | Test |
| FR-029-AC-8 | The key scan behind the in-flight and dead-letter lists matches only keys under the literal prefix even when it holds glob characters, returns at most its key bound, stops at its call bound, and says when it was cut; a machine named to the heartbeat read is found even when the scan is full. | Test |
| FR-029-AC-9 | Given only a dead letter, `agent-msg doctor` exits 0 and reports `warn` with the `agent-msg inbox --dead` hint; with `--strict` it exits 1. | Test |
| FR-029-AC-10 | Given a closed output pipe and a healthy relay, `agent-msg doctor` exits 0. | Test |
| FR-029-AC-11 | Given a stuck inbox, the code is a `warn` under a fresh heartbeat and a `fail` under a wedged or missing one; given a spool with only undecodable lines, `hook_spool_pending` is a `fail` while the file is being written and a `warn` after 15 min. | Test |
| FR-029-AC-12 | Given `node-a`'s watcher died and its leads' stamps stopped moving, `watcher_heartbeat_missing` for `node-a` is a `fail` half an hour, three hours and 23 hours after its last tick, and a `warn` after 25 hours. | Test |
| FR-029-AC-13 | Given the bus refuses the password (including `NOAUTH` and `WRONGPASS`), refuses the connection, accepts and then drops it, does not answer, or has no route, `redis_unreachable`'s cause and fix name, respectively, the machine's ACL user, hub's forwarder, relay-redis behind the forwarder, the WireGuard link, and WireGuard not being up on this machine. | Test |
| FR-029-AC-14 | Given one hook failure two minutes old, or 49 failures spread over several machines within 15 minutes, or a burst of 100 failures from one session within a single 15-minute window, `hook_failures_recorded` is a `warn`; given 50 failures under 15 minutes old, it is a `fail`. | Test |
| FR-029-AC-15 | Given one session that failed 84 times with the same stage and error kind over the last hour, in at least three distinct 15-minute windows and most recently three minutes ago, `hook_failures_recorded` is a `fail`; given the same records with the newest 20 minutes old it is a `warn`. | Test |
| FR-029-AC-16 | Given failures of several sessions on several machines, the check's detail names the three sessions and the three machines with the most failures, with their counts, repeating stage and error kind and newest time, and no text taken from a failure message. | Test |

## Dependencies

- **Upstream**: [US-006](../usecase/US-006-see-when-the-relay-is-failing.md)
- Reads [FR-028](./FR-028-watcher-heartbeat.md)'s heartbeat and runs
  [FR-036](./FR-036-herdr-version-and-shape-check.md)'s check; reads the in-flight
  and dead-letter lists of [FR-040](./FR-040-redelivery-is-marked-bounded-and-dead-lettered.md).
- Delivered by AGE-2033. `doctor` is for the operator: nothing it prints is added
  to what an agent sees. `doctor` reads Redis first and then runs the child
  processes, so a child exiting does not overlap a Redis wait (a child exiting
  while another thread waited on the Redis socket interrupted that wait in a
  process test: 7 of 400 and 6 of 600 runs with Redis read in parallel with the
  children, 0 of 600 with `SIGCHLD` blocked in the Redis thread or with Redis
  read first). `relay-tui` had the same exposure: its poll thread waits on a
  Redis socket beside a thread that spawns `herdr` and `linear`. AGE-2027 phase B
  repeats a poll that a signal interrupted (`Error::is_interrupted`, up to three
  attempts on a fresh connection; the poll only reads) rather than lock the poll
  across every child, which would freeze the list for the two minutes
  `agent start` may take. Tests:
  `a_poll_interrupted_by_a_signal_is_tried_again_up_to_its_bound` (the retry, its
  bound, and that a reset is not retried) and `only_an_interrupted_call_is_interrupted`
  (the classification, against a real `RedisError`); a real signal into a blocked
  read cannot be raised without `unsafe`, so the race itself is not reproduced.
