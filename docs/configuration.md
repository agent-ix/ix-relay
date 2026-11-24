# Configuration

`agent-msg`, `relay-watcher` and `relay-tui` read machine-level settings from `~/.ix/relay/config.toml`. Every key is optional, and a missing file means the compiled-in defaults. The file is the normal place to set a machine's bus URL, its name on the bus and its watcher options.

```toml
url = "redis://<machine>:<password>@10.99.0.1:6380"   # from `relay-admin add-peer`
machine = "<machine>"       # fixes hostname casing or a domain suffix
prefix = "ixr:v3:"
human = "operator"          # who escalate and resolve notify and sign as

[watcher]
interval_secs = 2
delivery = "herdr"          # or "socket"
# otlp_endpoint = "http://collector:4318"
# otlp_headers = "Authorization=Bearer%20token"
# otlp_interval_secs = 30

# [fleet]
# expected_sources = ["cpu", "memory", "temperature", "pressure", "capacity", "compilers", "pane_tail", "hook_events", "activity_counters", "herdr", "redis"]
# unavailable_sources = ["compilers"]
# working_window_secs = 600
# no_events_after_secs = 1800
# max_tool_runtime_secs = 7200
# ticket_prefixes = ["SWM", "QSL"]
```

## Keys

| Key | Meaning | Default |
|---|---|---|
| `url` | Redis URL of the bus | `redis://127.0.0.1:6379` |
| `machine` | The machine's name on the bus | the hostname |
| `prefix` | Redis key prefix | `ixr:v3:` |
| `human` | The human that `escalate` and `resolve` notify and sign as | `operator` |
| `[watcher] interval_secs` | How soon the watcher retries a lead that has mail but was left for its hook or a dialog | `2` |
| `[watcher] delivery` | `herdr` types batches in. `socket` posts to Claude Code's inbox socket for Claude leads. See [how it works](how-it-works.md#delivery-through-the-claude-code-inbox-socket). | `herdr` |
| `[watcher] otlp_endpoint` | OTLP/HTTP collector base URL, `http://` only. Unset sends nothing. | unset |
| `[watcher] otlp_headers` | Request headers, `name=value,name=value` | none |
| `[watcher] otlp_interval_secs` | Seconds between pushes, 5 to 3600 | `30` |
| `[fleet] expected_sources` | Collector sources this host provides, each exported as `ix_fleet_probe_ok`. A source not listed, or listed in `unavailable_sources`, is exported as `ix_fleet_probe_unavailable`. A host with a readable hwmon sensor adds `temperature`. | every source except `temperature` (and `pressure` off Linux) |
| `[fleet] unavailable_sources` | Sources this host cannot provide, for example `compilers` on a host with no process list. | none |
| `[fleet] working_window_secs` | Seconds a `prompt` or `post_tool` hook event keeps a lane `working`. Exported as `ix_fleet_config{name="working_window"}`. | `600` |
| `[fleet] no_events_after_secs` | Seconds a lane that was working may emit no hook event before it carries `ix_fleet_lane_no_events` and counts against `hook_events`. | `1800` |
| `[fleet] max_tool_runtime_secs` | Seconds an open `pre_tool` may run before the lane is `stuck`. | `7200` |
| `[fleet] ticket_prefixes` | Team keys read as a ticket in a lane's branch or working directory for `ix_fleet_lane_focus_info`. | AGE, IR, EA, IXD, JEV, OBS, QSL, PLAT, RES, SAPHO, STD, SWM, TL, VER |

Upgrade `agent-msg`, the hook and `relay-watcher` on a machine before adding `[fleet]` to its `config.toml`: a binary built before this table existed refuses a file that has it.

## Environment variables and flags

Each setting resolves in this order: flag, environment variable, file, default. A `--url` flag beats `IX_RELAY_URL`, which beats `url` in the file, which beats the compiled-in default.

| Variable | Setting |
|---|---|
| `IX_RELAY_URL` | `url` |
| `IX_RELAY_PREFIX` | `prefix` |
| `IX_RELAY_MACHINE` | `machine` |
| `IX_RELAY_HUMAN` | `human` |
| `IX_RELAY_WATCHER_INTERVAL_SECS` | `[watcher] interval_secs` |
| `IX_RELAY_WATCHER_OTLP_ENDPOINT` | `[watcher] otlp_endpoint` |
| `IX_RELAY_WATCHER_OTLP_HEADERS` | `[watcher] otlp_headers` |
| `IX_RELAY_WATCHER_OTLP_INTERVAL_SECS` | `[watcher] otlp_interval_secs` |
| `IX_RELAY_CONFIG` | Path of the config file, used by tests and alternate installs |

The flags are `--url`, `--prefix` and `--human` on `agent-msg` and `relay-tui`, and `--url`, `--prefix`, `--interval-secs`, `--sweep-secs` and `--once` on `relay-watcher`. `--sweep-secs` sets the backup sweep period, default 15 s, and has no file key.

`AGENT_MSG_NAME` is left in the session environment and takes no part in identity or routing.

## Errors in the file

A malformed file is an error that names the file, line and column. `agent-msg hook` reports it on stderr and still exits 0, so a typo never breaks a lead's turn. Every other command exits 1. A bad OTLP value stops `relay-watcher` at start-up with exit 1, and `agent-msg` and the hooks ignore it. `agent-msg doctor` then warns `watcher_otlp_config_invalid`.

## Files

| Path | Content |
|---|---|
| `~/.ix/relay/config.toml` | The settings above. Use mode 600, because `url` holds a password. |
| `~/.ix/relay/spool/hook.jsonl` | Hook failure records waiting to be shipped, capped at 256 KiB. |
| `~/.ix/relay/state/watcher.json` | The watcher's `pid`, `started_at_ms`, `last_ok_tick_ms` and `last_error`. `relay-watcher --once` never writes it. |
| `~/.ix/relay/state/socket/socket-<pid>.json` | Socket delivery status for one watcher, in a directory with mode 0700. |
| `~/.ix/relay/peers/<machine>.url` | On the hub: the bus URL for a machine, mode 600, written by hand for transfer. |

State and spool files sit beside the config file, or beside the path in `IX_RELAY_CONFIG`.

## Redis keys

All keys start with the prefix. The Redis user of each machine can touch only `ixr:*` keys. Key families include the session inbox, `inflight:<lead>`, `deadletter:<lead>`, the message log stream, the capped `health:hooks` stream, `health:watcher:<machine>` heartbeats, the directory snapshots and the escalations.
