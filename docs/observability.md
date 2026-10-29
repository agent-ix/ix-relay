# Observability

`relay-watcher` can push metrics over OTLP/HTTP. Claude Code labels its own OpenTelemetry metrics with Herdr's pane, tab and workspace IDs, which carry no names. The watcher's metrics let a dashboard join those IDs to bus names and keep the name a pane had at the time. The watcher also pushes a few relay health series. With no endpoint configured the watcher sends nothing, and it binds no listening socket in either case.

## Enable the push

Set an endpoint in `~/.ix/relay/config.toml`:

```toml
[watcher]
otlp_endpoint = "http://<collector>/api/v1/otlp"   # a base URL; /v1/metrics is appended
otlp_interval_secs = 30
```

The same settings exist as `IX_RELAY_WATCHER_OTLP_ENDPOINT`, `IX_RELAY_WATCHER_OTLP_HEADERS` and `IX_RELAY_WATCHER_OTLP_INTERVAL_SECS`. The interval is 5 to 3600 seconds, and any other value is refused. Restart the watcher after a change. `relay-watcher --once` sends nothing.

The body is OTLP JSON (`Content-Type: application/json`, `POST <endpoint>/v1/metrics`). The collector must accept OTLP/HTTP JSON on its HTTP receiver, usually port 4318. The gRPC port 4317 does not work. No TLS library is built in, so an `https://` endpoint in the relay's own settings is refused at start-up with a message naming the key or variable.

A minimal OpenTelemetry Collector for receiving the metrics:

```yaml
receivers:
  otlp:
    protocols:
      http:
        endpoint: 0.0.0.0:4318
exporters:
  prometheus:
    endpoint: 0.0.0.0:8889
service:
  pipelines:
    metrics:
      receivers: [otlp]
      exporters: [prometheus]
```

## Precedence

The relay's own settings come first. Headers always come from the same source as the endpoint, so credentials meant for one collector never reach another.

| Endpoint from | Headers from |
|---|---|
| `IX_RELAY_WATCHER_OTLP_ENDPOINT` | `IX_RELAY_WATCHER_OTLP_HEADERS` |
| `[watcher] otlp_endpoint`, when the variable is unset | `[watcher] otlp_headers` |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` as written, or else `OTEL_EXPORTER_OTLP_ENDPOINT` with `/v1/metrics` appended, when neither source above is set | `OTEL_EXPORTER_OTLP_HEADERS` |
| none | none, and nothing is sent |

The generic `OTEL_*` variables belong to other emitters on the host, and Claude Code sets them for its own telemetry. The watcher honors them only as the last fallback, only for an `http://` endpoint, and only when `OTEL_EXPORTER_OTLP_(METRICS_)PROTOCOL` is unset or `http/json`. Otherwise it ignores them, logs one line naming the variable and the reason, and `agent-msg doctor` shows `otlp: disabled (VAR ignored: reason)`.

A bad value in the relay's own settings stops `relay-watcher` at start-up with exit 1. The message names the variable, or the file's key with its line and column, and never a header value. `agent-msg` and the hooks ignore the bad value, so delivery continues, and `doctor` warns `watcher_otlp_config_invalid`. Set the endpoint in the config file so that the generic variables are never consulted.

The resource attributes are `service.name`, `host.name` (the machine's bus name), `service.version` and any pairs from `OTEL_RESOURCE_ATTRIBUTES`. `service.name` comes from `OTEL_SERVICE_NAME`, then from `OTEL_RESOURCE_ATTRIBUTES`, then defaults to `ix-relay-watcher`.

## Series

No series carries message text.

| Name | Kind | Unit | Attributes |
|---|---|---|---|
| `herdr.pane.info` | gauge (1) | `1` | machine, pane, tab, workspace, workspace_label, tab_label, session, bus_name |
| `herdr.pane.status` | gauge (1) | `1` | machine, pane, status |
| `ix.relay.messages` | sum | `{message}` | outcome: delivered, requeued, redelivered, dead_lettered |
| `ix.relay.inbox.depth` | gauge | `{message}` | lead |
| `ix.relay.inbox.oldest_age` | gauge | `s` | lead |
| `ix.relay.inflight.entries` | gauge | `{message}` | lead |
| `ix.relay.deadletter.entries` | gauge | `{message}` | lead |
| `ix.relay.hook.failures.recent` | gauge | `{failure}` | stage, error_kind |
| `ix.relay.watcher.tick.duration` | histogram | `s` | none |
| `ix.relay.registrations` | gauge | `{registration}` | machine |

- Each watcher publishes its own machine's panes only, one series per Herdr pane that has an agent. `herdr.pane.status` joins `herdr.pane.info` on `(machine, pane)`. `bus_name` is empty for a pane that no lead registered.
- Label values are cleaned of control, bidirectional and invisible characters, cut to 128 characters and JSON-escaped. Panes come from the layout the watcher reads about every 30 s, so a renamed tab appears within one refresh. If Herdr cannot be read, the last panes are kept for 90 s and then left out.
- Counters are cumulative since the watcher started. `ix.relay.messages` counts what this watcher did or saw: `delivered` means typed into a pane by the watcher, and the other outcomes are returns and dead letters from expired leases.
- `lead` is bounded. At most 50 leads have series of their own and the rest fold into `lead=other`. `agent-msg doctor` reports how many.
- `ix.relay.hook.failures.recent` is a gauge of this machine's failures still in the retained `health:hooks` stream since the watcher started. It falls when the capped stream drops old entries.

Once per push, the tick adds two pipelined Redis requests after its deliveries: one over the local leads' own keys and one for the newest hook failures, up to 1,000 entries. There is no key scan.

## Behavior when the collector fails

The push runs on its own thread, fed by a queue of two snapshots, and the oldest is dropped when the queue is full. A slow or dead collector never delays a tick, grows memory or stops the watcher.

- The connect timeout is 2 s, the whole request is 5 s, redirects are not followed and a request is at most 4 MiB.
- After a failure it waits 1 s, doubling up to 5 minutes, or the collector's `Retry-After` up to 5 minutes.
- A 4xx response other than 429 refuses that request and causes no back-off. A 2xx response that rejects every point is a failure (`rejected_points`).
- One log line starts an outage and one ends it.

`agent-msg doctor` shows `otlp: sending to <host:port>` with the exporter's counters (pushes sent and failed, snapshots dropped, points rejected), `otlp: disabled` or `otlp: unknown`. It warns `watcher_otlp_failing` with a reason such as `connect_failed`, `timeout`, `http_503`, `rejected_points` or `stuck` when the collector cannot be reached. A name lookup has no deadline of its own, so a send that runs over 30 s shows as `stuck`. Header values and endpoint credentials are never printed.

## Agent spend dashboard

The Helm chart in `helm/agent-spend-dashboard` installs a `grafana-dashboards` ConfigMap holding the Grafana dashboard with UID `agent-spend`. Grafana reads it from the observability deployment. The dashboard shows cost, tokens, sessions and active agents by server, project, `machine:workspace:tab`, model and account. Both the native clients and the watcher push metrics, so add no scrape targets for Herdr machines.

### Add a machine to the dashboard

Make the smallest additive edit to each file, and keep existing keys, headers and exporter settings. Do not print secrets or copy credentials between machines.

1. Find the Prometheus ingress host in the observability deployment's configuration. It is `prometheus.<internalBaseDomain>`, and the base domain comes from the deployment's values.
2. Set the watcher's endpoint to `http://prometheus.<internalBaseDomain>/api/v1/otlp`, as shown above. Do not use the Prometheus web UI root or port 9090.
3. Enable Claude Code's OTLP metrics exporter in `~/.claude/settings.json` under `env`: set `CLAUDE_CODE_ENABLE_TELEMETRY`, enable the metrics exporter, set `OTEL_EXPORTER_OTLP_PROTOCOL` to `http/protobuf` and set the metrics endpoint to `http://prometheus.<internalBaseDomain>/api/v1/otlp/v1/metrics`.
4. Enable Codex's OTLP HTTP metrics exporter in `~/.codex/config.toml` under `[otel]`, with the same metrics endpoint. Use the field names that the installed Codex version accepts.
5. Restart the watcher if its configuration changed. Claude Code and Codex read telemetry settings when a process starts, so new sessions pick them up.
6. Check `agent-msg doctor` and the watcher log for endpoint errors. Confirm ingestion in Prometheus with `herdr_pane_info_ratio{machine="<machine>"}`, then open the `agent-spend` dashboard and select the machine.

### Troubleshoot

- No `herdr_pane_info_ratio` for the machine: check the watcher process, the resolved endpoint, network reachability to the Prometheus ingress and the watcher log.
- The machine appears and the token charts are empty: confirm native telemetry is enabled in the Claude Code or Codex configuration, start a new session and generate usage. Watcher pane metrics create no token measurements. Token series appear only after recent usage.
- HTTP 404: confirm that `/api/v1/otlp` is the Prometheus OTLP receiver route in the active observability deployment.
