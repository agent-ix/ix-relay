# CLI reference

This page is generated from the `--help` output of the built binaries (version 0.3.4). Run `<command> --help` for the same text. Every `agent-msg` subcommand also accepts `--url`, `--prefix` and `--human`; [Configuration](configuration.md) describes them.

## agent-msg

`agent-msg` is the lead's command-line client and the Claude and Codex hook handler.

```text
Send and read messages between team leads.

Usage: agent-msg [OPTIONS] <COMMAND>

Commands:
  work             Report session scope, focus and agent assignments for the dashboard
  send             Send a message to one lead, or to `all`
  inbox            Print and remove your pending messages
  who              List registered leads
  focus            Set or clear this lead's lane focus
  server-capacity  Manage operator-entered build-server capacity
  status-request   Ask selected registered lanes for the fixed program-review status form
  status-reply     Submit one structured response to a status request
  status-replies   Return structured replies and lanes that have not responded yet
  find             Look up a live lead by address, workspace/tab label, session name, pane id or `server:workspace[:tab[:pane]]` address, joining the bus registry against live herdr state on this machine and every saved one. Ends with the address(es) to use with `send`. Not needed before `send`: `send` resolves an address itself
  doctor           Check the relay's health: Redis, this machine's watcher, hook failures, herdr, registrations, queue depth and the watcher's unit. Read-only; works with Redis down. Exits 1 if anything is wrong
  log              Show recent message history
  register         Register yourself by hand. Hooks do this on session start
  whoami           Print this session's verified server:workspace:tab identity. An unverified session prints not a lead; aliases never supply identity
  hook             Hook handler: reads hook JSON on stdin, prints hook output. Never fails the hook
  escalate         File an escalation: a decision only the operator can make. Prints its id
  escalations      List escalations
  investigate      Open a Claude Code session on an escalation's lead's machine, in its cwd, to look into it
  resolve          Record a decision, tell the lead, and post it to the ticket
  escalation       Read or act on one escalation's full detail
  help             Print this message or the help of the given subcommand(s)

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
  -V, --version          Print version
```

### agent-msg send

```text
Send a message to one lead, or to `all`

Usage: agent-msg send [OPTIONS] <TO> <TEXT>...

Arguments:
  <TO>       Full server:workspace:tab, :workspace:tab, :tab, or all. The relay discovers and enrolls the live Herdr lead automatically
  <TEXT>...  The message

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg inbox

```text
Print and remove your pending messages

Usage: agent-msg inbox [OPTIONS]

Options:
      --max <MAX>        Most messages to take [default: 50]
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --json             Print `{"messages": [...], "rejected": [...]}` instead of plain text, for callers that parse the output (e.g. the opencode plugin). Still pops: use `--count` for a non-destructive peek
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --count            Print the number of pending messages without removing anything, so a caller can decide whether it's worth starting a turn to read them (the opencode plugin's idle path does this)
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
      --dead             Print your dead letters (messages whose redeliveries ran out) instead of popping, oldest first, and leave them where they are
      --in-flight        Print, for every lead that has some, how many messages are in flight (popped, not yet acknowledged) and how many are dead letters, found by scanning the keys: it lists leads that were pruned or renamed too, whose stranded batches nothing else shows. Looks at no more than 1,000 keys of each kind and says when it stopped there. Changes nothing
  -h, --help             Print help
```

### agent-msg who

```text
List registered leads

Usage: agent-msg who [OPTIONS]

Options:
      --focus                      Include each lead's explicitly reported focus and freshness
      --url <URL>                  Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>            Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --servers                    Group registered lanes by server and show operator-set capacity
      --human <HUMAN>              The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
      --stale-after <STALE_AFTER>  Mark focus older than this age as stale (for example `7d` or `12h`) [default: 7d]
  -h, --help                       Print help
```

### agent-msg find

```text
Look up a live lead by address, workspace/tab label, session name, pane id or `server:workspace[:tab[:pane]]` address, joining the bus registry against live herdr state on this machine and every saved one. Ends with the address(es) to use with `send`. Not needed before `send`: `send` resolves an address itself

Usage: agent-msg find [OPTIONS] [QUERY]

Arguments:
  [QUERY]  Case-insensitive substring to match; omit to list everyone

Options:
      --json             Print one JSON object per matching row instead of plain text
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg log

```text
Show recent message history

Usage: agent-msg log [OPTIONS]

Options:
      --since <SINCE>    How far back, e.g. 30m, 1h, 2d [default: 1h]
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --max <MAX>        Most messages to show [default: 100]
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg doctor

```text
Check the relay's health: Redis, this machine's watcher, hook failures, herdr, registrations, queue depth and the watcher's unit. Read-only; works with Redis down. Exits 1 if anything is wrong

Usage: agent-msg doctor [OPTIONS]

Options:
      --json             Print one JSON object `{"ok": ..., "checks": [...]}` instead of text. Each check has a stable `code`
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --strict           Exit 1 on a warning too (the default fails only on a failure)
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg register

```text
Register yourself by hand. Hooks do this on session start

Usage: agent-msg register [OPTIONS]

Options:
      --kind <KIND>      Harness kind [default: manual]
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg whoami

```text
Print this session's verified server:workspace:tab identity. An unverified session prints not a lead; aliases never supply identity

Usage: agent-msg whoami [OPTIONS]

Options:
      --json             Print the canonical address and location source, or {"lead":false}
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg hook

```text

```

### agent-msg escalate

```text
File an escalation: a decision only the operator can make. Prints its id

Usage: agent-msg escalate [OPTIONS] --context <CONTEXT> --recommend <RECOMMEND> <TICKET>

Arguments:
  <TICKET>  The Linear ticket this decision belongs to, e.g. `ENG-123`

Options:
      --context <CONTEXT>      What needs deciding
      --url <URL>              Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --option <OPTION>        A choice the operator could make. Repeatable
      --prefix <PREFIX>        Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>          The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
      --recommend <RECOMMEND>  Your own recommendation
  -h, --help                   Print help
```

### agent-msg escalations

```text
List escalations

Usage: agent-msg escalations [OPTIONS]

Options:
      --status <STATUS>
          Only escalations in this state. Defaults to every state

          Possible values:
          - waiting:       Filed; nobody has started looking at it yet
          - investigating: Someone has a session open on it
          - resolved:      A decision was recorded

      --url <URL>
          Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`

      --prefix <PREFIX>
          Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]

      --human <HUMAN>
          The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`

  -h, --help
          Print help (see a summary with '-h')
```

### agent-msg investigate

```text
Open a Claude Code session on an escalation's lead's machine, in its cwd, to look into it

Usage: agent-msg investigate [OPTIONS] <ID>

Arguments:
  <ID>  The escalation id, e.g. `e7`

Options:
      --fresh            Start a fresh session with a brief prompt instead of forking the lead's own. The full context, options and recommendation are read with `agent-msg escalation show <id>` from inside that session — never typed into it directly
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg resolve

```text
Record a decision, tell the lead, and post it to the ticket

Usage: agent-msg resolve [OPTIONS] <ID> <DECISION>

Arguments:
  <ID>        The escalation id, e.g. `e7`
  <DECISION>  The decision

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg escalation

```text
Read or act on one escalation's full detail

Usage: agent-msg escalation [OPTIONS] <COMMAND>

Commands:
  show         Print one escalation's full detail: ticket, lead, machine, cwd, kind, status, context, options and recommendation. This is what `investigate --fresh`'s prompt points a fresh session at, rather than embedding that text directly (see `escalate.rs`'s module doc comment)
  post-linear  Retry posting a resolved escalation's decision to its Linear ticket — for when `resolve`'s own attempt failed
  help         Print this message or the help of the given subcommand(s)

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg escalation show

```text
Print one escalation's full detail: ticket, lead, machine, cwd, kind, status, context, options and recommendation. This is what `investigate --fresh`'s prompt points a fresh session at, rather than embedding that text directly (see `escalate.rs`'s module doc comment)

Usage: agent-msg escalation show [OPTIONS] <ID>

Arguments:
  <ID>  The escalation id, e.g. `e7`

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg escalation post-linear

```text
Retry posting a resolved escalation's decision to its Linear ticket — for when `resolve`'s own attempt failed

Usage: agent-msg escalation post-linear [OPTIONS] <ID>

Arguments:
  <ID>  The escalation id, e.g. `e7`

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg focus

```text
Set or clear this lead's lane focus

Usage: agent-msg focus [OPTIONS] [FOCUS]

Arguments:
  [FOCUS]  The current project, layer, or ticket focus

Options:
      --clear            Clear this lane's focus
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg server-capacity

```text
Manage operator-entered build-server capacity

Usage: agent-msg server-capacity [OPTIONS] <COMMAND>

Commands:
  set    Record explicit capacity for one server
  clear  Remove explicit capacity for one server
  help   Print this message or the help of the given subcommand(s)

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg server-capacity set

```text
Record explicit capacity for one server

Usage: agent-msg server-capacity set [OPTIONS] --cores <CORES> --ram-gib <RAM_GIB> <SERVER>

Arguments:
  <SERVER>  Relay server name, such as the machine name in `agent-msg who`

Options:
      --cores <CORES>             Cores allocated to agent work
      --url <URL>                 Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>           Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --ram-gib <RAM_GIB>         RAM in GiB
      --human <HUMAN>             The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
      --max-agents <MAX_AGENTS>   Optional maximum number of agents
      --owner-rule <OWNER_RULES>  Operator-entered constraints, such as `Kani team only`
  -h, --help                      Print help
```

### agent-msg server-capacity clear

```text
Remove explicit capacity for one server

Usage: agent-msg server-capacity clear [OPTIONS] <SERVER>

Arguments:
  <SERVER>  Relay server name

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg status-request

```text
Ask selected registered lanes for the fixed program-review status form

Usage: agent-msg status-request [OPTIONS] <LANES>

Arguments:
  <LANES>  `all`, or comma-separated full lane addresses

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg status-reply

```text
Submit one structured response to a status request

Usage: agent-msg status-reply [OPTIONS] --state <STATE> --spare-coder-slots <SPARE_CODER_SLOTS> <REQUEST_ID>

Arguments:
  <REQUEST_ID>  Request ID from `status-request`

Options:
      --ticket <TICKETS>
          Current Linear ticket IDs; repeat for multiple tickets
      --url <URL>
          Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>
          Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --pull-request <PULL_REQUESTS>
          Current pull request URLs or IDs; repeat for multiple PRs
      --blocker-ticket <BLOCKER_TICKET_IDS>
          Blocking Linear ticket IDs; repeat for multiple blockers
      --human <HUMAN>
          The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
      --state <STATE>
          Lane state: active, idle, or blocked [possible values: active, idle, blocked]
      --spare-coder-slots <SPARE_CODER_SLOTS>
          Number of coder slots currently free in this lane
      --owner-decision <OWNER_DECISIONS>
          Decisions required from the project owner; repeat if needed
  -h, --help
          Print help
```

### agent-msg status-replies

```text
Return structured replies and lanes that have not responded yet

Usage: agent-msg status-replies [OPTIONS] <REQUEST_ID>

Arguments:
  <REQUEST_ID>  Request ID from `status-request`

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg work

```text
Report session scope, focus and agent assignments for the dashboard

Usage: agent-msg work [OPTIONS] <COMMAND>

Commands:
  scope   Set whole-team scope or resolve narrower selectors against Linear
  focus   Report the lead's current focus and optionally related tickets
  agent   Report one agent's assignment or completion without marking a ticket done
  remove  Remove a finished assignment from this session's bounded report
  show    Print the current session-qualified report as structured JSON
  help    Print this message or the help of the given subcommand(s)

Options:
      --lead <LEAD>        Target lead's Herdr address; omitted means the calling pane
      --url <URL>          Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>    Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --session <SESSION>  Expected native session ID; refuse a different occupant
      --human <HUMAN>      The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help               Print help
```

### agent-msg work scope

```text
Set whole-team scope or resolve narrower selectors against Linear

Usage: agent-msg work scope [OPTIONS]

Options:
      --all-team                  Clear narrower selectors and monitor the lead's whole team
      --url <URL>                 Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --initiative <INITIATIVES>  Initiative UUID or exact name; repeatable
      --prefix <PREFIX>           Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>             The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
      --project <PROJECTS>        Project UUID, identifier or exact name; repeatable
      --ticket <TICKETS>          Ticket UUID or identifier, including descendants; repeatable
  -h, --help                      Print help
```

### agent-msg work focus

```text
Report the lead's current focus and optionally related tickets

Usage: agent-msg work focus [OPTIONS] <SUMMARY>

Arguments:
  <SUMMARY>  Short plain-language action

Options:
      --ticket <TICKETS>  Ticket identifier; repeatable
      --url <URL>         Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>   Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>     The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help              Print help
```

### agent-msg work agent

```text
Report one agent's assignment or completion without marking a ticket done

Usage: agent-msg work agent [OPTIONS] --ticket <TICKET> --role <ROLE> --status <STATUS> <ID>

Arguments:
  <ID>  Native agent ID, or stable assignment ID for a pane worker

Options:
      --name <NAME>      Display name; defaults to the assignment ID
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --ticket <TICKET>  Ticket identifier
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
      --role <ROLE>      Role such as coder, reviewer or researcher
      --status <STATUS>  Reported status: running, waiting, done, failed or canceled
      --worker <WORKER>  Herdr address for a separate pane worker; captures its native session
  -h, --help             Print help
```

### agent-msg work remove

```text
Remove a finished assignment from this session's bounded report

Usage: agent-msg work remove [OPTIONS] <ID>

Arguments:
  <ID>  Agent assignment ID to remove

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

### agent-msg work show

```text
Print the current session-qualified report as structured JSON

Usage: agent-msg work show [OPTIONS]

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Lets tests (and, in principle, separate buses) share one Redis without colliding. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `escalate`/`resolve` notify/sign as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
  -h, --help             Print help
```

## relay-watcher

`relay-watcher` runs one process per machine and delivers queued messages to idle leads.

```text
Wakes idle team leads on this machine by typing ix-relay messages in through herdr.

Usage: relay-watcher [OPTIONS]

Options:
      --url <URL>                      Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>                Redis key prefix, matching `agent-msg`'s. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --interval-secs <INTERVAL_SECS>  How soon to look again at a lead that has messages queued but was left for its own hook (mid-turn, blocked). New messages wake the watcher at once whatever this is. Defaults come from `~/.ix/relay/config.toml`'s `[watcher] interval_secs`, then `IX_RELAY_WATCHER_INTERVAL_SECS`, then 2
      --sweep-secs <SWEEP_SECS>        How long after everything was last looked at the backup sweep looks again, in seconds. Sends wake the watcher at once, so this only bounds the cost of a lost wake and the age of the state file [default: 15]
      --once                           Run a single sweep and exit, instead of looping forever. For manual checks and tests; the systemd/launchd units never pass this. Exits nonzero if that one sweep couldn't even reach Redis
  -h, --help                           Print help
  -V, --version                        Print version
```

## relay-tui

`relay-tui` is the operator's escalation inbox.

```text
The operator's escalation inbox: investigate and resolve ix-relay escalations.

Usage: relay-tui [OPTIONS]

Options:
      --url <URL>        Redis URL of the bus. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_URL`, then `redis://127.0.0.1:6379`
      --prefix <PREFIX>  Redis key prefix. Defaults come from `~/.ix/relay/config.toml`, then `IX_RELAY_PREFIX`, then [`relay_core::DEFAULT_PREFIX`]
      --human <HUMAN>    The human inbox `resolve` notifies/signs as. Defaults come from `~/.ix/relay/config.toml`'s `human` key, then `IX_RELAY_HUMAN`, then `operator`
      --notify           Fire a `herdr notification show` once per newly arrived waiting escalation (never for the backlog already waiting when `relay-tui` starts — see `app.rs`'s new-arrival detection). Off by default: on a machine with no desktop-notification support, this would just be a failing herdr call every time a new escalation shows up
  -h, --help             Print help
  -V, --version          Print version
```

## team-leader-dashboard

`team-leader-dashboard` monitors one lead session. It is built from the `relay-tui` crate.

```text
Monitor one explicitly assigned lead session

Usage: team-leader-dashboard [OPTIONS] --lead <LEAD>

Options:
      --lead <LEAD>        Lead Herdr address, e.g. hub:TEAM:code
      --team <TEAM>        Linear team key; defaults to the bound lead workspace label
      --session <SESSION>  Native session expected at that address
  -h, --help               Print help
```

## relay-admin

`scripts/relay-admin` runs on the hub and manages the bus's transport. It is a shell script and has no `--help`.

| Command | Effect |
|---|---|
| `relay-admin hub-init [--lan CIDR]` | Creates `wg0` and its firewall once. Needs `sudo`. |
| `relay-admin add-peer <machine> <pubkey>` | Adds a WireGuard peer and a Redis user, and prints the machine's WireGuard config and bus URL. |
| `relay-admin remove-peer <machine>` | Removes the peer and its Redis user. |
| `relay-admin add-user <machine>` | Adds or reissues a Redis user only, such as the hub's own, and prints its URL. |
| `relay-admin remove-user <machine>` | Removes a Redis user. |
| `relay-admin migrate` | Copies `ixr:v3:*` keys from `platform/redis` into relay-redis. Existing keys are kept. |
| `relay-admin status` | Shows `wg0`, the forwarder and relay-redis. |
