---
name: agent-observability-setup
description: Configure and troubleshoot Claude Code, Codex, and ix-relay watcher metrics for the Agent IX Grafana agent-spend dashboard on a machine. Use when adding a Herdr server to agent spend or investigating missing server or token data.
---

# Agent spend telemetry setup

Use this procedure when Claude Code or Codex on a Herdr machine is missing from the agent-spend dashboard. This is agent-performed host configuration: inspect the machine's current settings, make the smallest additive edits, then verify telemetry. The setup is push based; do not add the machine to Prometheus scrape targets.

## 1. Inspect the machine and existing configuration

1. Confirm the Herdr server name and the active workspace, then inspect the machine's existing Claude Code, Codex, and ix-relay watcher configuration. On a remote machine, use its configured Herdr/SSH access. Do not assume that Terra's configuration applies to another server.
2. Read `~/.claude/settings.json`, `~/.codex/config.toml`, and `~/.ix/relay/config.toml` if present. Preserve existing keys, unrelated environment variables, auth headers, and exporter settings. Do not print secrets into chat or logs.
3. Check whether relay-watcher is installed and running (`systemctl --user status relay-watcher` on Linux; `launchctl list` for `com.agent-ix.relay-watcher` on macOS). Record current state before changing anything.
4. Determine the active Prometheus ingress host from the deployed `ix-local-observability` configuration or cluster ingress. Its hostname is `prometheus.<internalBaseDomain>`; discover the configured base domain from deployment values. Do not infer it from the Herdr server name or hardcode `luna.ix`.

## 2. Configure the relay watcher

Add or update only the `[watcher]` keys in `~/.ix/relay/config.toml`:

```toml
[watcher]
otlp_endpoint = "http://prometheus.<internalBaseDomain>/api/v1/otlp"
otlp_interval_secs = 30
```

Keep any existing `otlp_headers` if the deployed endpoint requires them. Do not copy collector credentials between machines. The endpoint is a base URL: relay-watcher appends `/v1/metrics` and sends OTLP/HTTP JSON. Do not set the endpoint to the Prometheus web UI root or port 9090.

## 3. Configure Claude Code and Codex

Enable each installed agent's native OTLP metrics exporter and point it at the same Prometheus OTLP receiver. Match the protocol expected by the receiver (`http/protobuf` for Claude Code and Codex); do not reuse relay-watcher's JSON protocol setting for native exporters. Use each application's documented config shape and preserve its existing settings. In particular:

- Claude Code configuration is in `~/.claude/settings.json` under `env`. Enable `CLAUDE_CODE_ENABLE_TELEMETRY`, enable the OTLP metrics exporter, set `OTEL_EXPORTER_OTLP_PROTOCOL` to `http/protobuf`, and set the metrics endpoint to `http://prometheus.<internalBaseDomain>/api/v1/otlp/v1/metrics`.
- Codex configuration is in `~/.codex/config.toml` under `[otel]` and its OTLP HTTP metrics exporter table. Enable metrics and set its endpoint to `http://prometheus.<internalBaseDomain>/api/v1/otlp/v1/metrics` using the installed Codex version's accepted config fields. Preserve existing trace and log choices.

Before writing, compare the configuration shape with known working Herdr machines (Terra, Draco, and Luna) where available. Adapt field names to the installed client version rather than replacing whole config files. Never copy machine identity or secrets from another server.

## 4. Restart and verify

- Restart relay-watcher only if its configuration changed. On Linux use `systemctl --user restart relay-watcher`; on macOS use `launchctl kickstart -k gui/$(id -u)/com.agent-ix.relay-watcher`.
- Claude Code and Codex read telemetry settings when a process starts. New sessions should use the updated settings; do not terminate active sessions unless requested.
- Check `agent-msg doctor` and watcher logs for OTLP endpoint/configuration errors. Confirm ingestion in Prometheus with `herdr_pane_info_ratio{machine="<server>"}`. Confirm available Claude usage series before diagnosing token data; token series appear only after recent usage and may be absent while the machine identity is already present.
- Open Grafana dashboard UID `agent-spend` and select the server. A machine can be present without recent token activity.

## Troubleshooting

- No `herdr_pane_info_ratio` for the server: verify watcher process, the resolved endpoint, network reachability to Prometheus ingress, and watcher logs.
- Machine appears but token charts are empty: verify native telemetry is enabled in the corresponding Claude Code/Codex process config, launch a new session, and generate usage. Relay-watcher pane metrics do not create token measurements.
- HTTP 404: verify `/api/v1/otlp` is the Prometheus OTLP receiver route from the active observability deployment.
- Do not add scrape targets for Herdr machines: both native clients and relay-watcher push metrics to the receiver.

## Source of truth

The dashboard definition and this setup procedure live in ix-relay. The versioned `agent-spend-dashboard` chart installs the `grafana-dashboards` ConfigMap consumed by Grafana in ix-local-observability. See the repository README's Metrics section for the metric catalogue and exporter behavior.
