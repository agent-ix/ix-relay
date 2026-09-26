---
id: FR-026
title: "Configuration precedence is flag, env, file, default"
type: FR
relationships:
  - target: "ix://agent-ix/ix-relay/US-005"
    type: "implements"
---
# FR-026: Configuration precedence is flag, env, file, default

## Description

When `agent-msg` or `relay-watcher` resolves a setting (`url`, `prefix`,
`machine`, `human`, `[watcher] interval_secs`, and the watcher's `otlp_endpoint`,
`otlp_headers` and `otlp_interval_secs` and the OpenTelemetry variables, which have no flag: [FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md)), the configuration loader shall
take the value from the command-line flag if given, else the environment
variable if set and non-empty, else `~/.ix/relay/config.toml` if the key is
present, else the compiled-in default.

## Behavior

- For the watcher's OTLP settings ([FR-045](./FR-045-watcher-pushes-metrics-over-otlp.md))
  the precedence is per tier, and headers follow their endpoint's tier:

  | Endpoint from | Headers from | Interval from |
  |---|---|---|
  | `IX_RELAY_WATCHER_OTLP_ENDPOINT` | `IX_RELAY_WATCHER_OTLP_HEADERS` | `IX_RELAY_WATCHER_OTLP_INTERVAL_SECS`, else the file, else 30 s |
  | `[watcher] otlp_endpoint` (no relay environment endpoint) | `[watcher] otlp_headers` | as above |
  | `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`, else `OTEL_EXPORTER_OTLP_ENDPOINT` (no relay endpoint anywhere; `http://` and http/json only, else ignored) | `OTEL_EXPORTER_OTLP_HEADERS` | as above |
  | none | none | as above |

- The configuration loader shall read the file at `IX_RELAY_CONFIG` when that
  variable is set, and `$HOME/.ix/relay/config.toml` otherwise.
- When the file does not exist, the configuration loader shall use the defaults
  (`redis://127.0.0.1:6379`, `ixr:`, the normalised hostname, `operator`, 2 seconds).
- The configuration loader shall treat a set-but-empty environment variable as
  unset.
- If the file sets `url`, `machine`, `prefix` or `human` to the empty string, or
  holds a key it does not know, then the configuration loader shall refuse the
  file as malformed.
- If the resolved machine name normalises to the empty string, then the
  configuration loader shall refuse it, whichever tier supplied it.
- If `IX_RELAY_WATCHER_INTERVAL_SECS` is set, non-empty and not an unsigned
  integer, then the configuration loader shall refuse it rather than fall through.
- `agent-msg` shall never read the watcher settings, so a bad watcher setting
  cannot fail it.
- `AGENT_MSG_NAME` shall not be a file key.

## Acceptance Criteria

| ID | Criteria | Verification |
|----|----------|--------------|
| FR-026-AC-1 | For every key and every combination of flag, environment and file values present or absent, the resolved value is the highest-precedence present value, else the default, and the reported source names that tier. | Test |
| FR-026-AC-2 | For every setting (`url`, `prefix`, `machine`, `human` and the watcher interval), given its environment variable set to the empty string, the resolved value is the file's value when the file has the key, else the default, and the reported source is that tier. | Test |
| FR-026-AC-3 | Given a file with `prefix = ""`, loading fails naming the file and the key `prefix`. | Test |
| FR-026-AC-4 | Given `IX_RELAY_MACHINE=.`, loading fails. | Test |
| FR-026-AC-5 | Given `IX_RELAY_WATCHER_INTERVAL_SECS=abc`, `relay-watcher` exits 1 and `agent-msg who` is unaffected. | Test |
| FR-026-AC-6 | Given a file with key `name = "x"`, loading fails as malformed. | Test |

## Dependencies

- **Upstream**: [US-005](../usecase/US-005-configure-a-machine-once.md)
- **Downstream**: [FR-027](./FR-027-malformed-configuration-per-command.md)
- Delivered by AGE-2017.
