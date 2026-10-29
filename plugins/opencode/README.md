# ix-relay opencode plugin

The ix-relay opencode plugin is the integration that connects a team lead's opencode session to the ix-relay bus. Messages from other leads appear in the model's context.

The plugin is for team leads. It skips any session that opencode marks as a subagent (`Session.parentID` set), and any session without a verified caller identity. A lead's identity is its `server:workspace:tab`. `AGENT_MSG_NAME` is left unchanged and does not affect identity.

The adapter checks identity when it loads and stays disabled if that proof is unavailable. Its inbox pop is destructive, so deliveries through this plugin are at-most-once. The [directory state model](../../spec/reviews/relay-directory-state-model.md) states the intended startup and delivery contract.

It was checked against opencode and `@opencode-ai/plugin` 1.17.11.

## Delivery points

| Point | Mechanism |
|---|---|
| Mid-turn | The `tool.execute.after` hook appends pending messages to the tool's `output.output`, and the harness sends that text to the model as the tool result. This is the main delivery point and corresponds to Claude Code's `PostToolUse` hook. |
| Turn end | opencode has no hook that blocks a turn from ending. On `session.idle`, the plugin runs `agent-msg inbox --count`, which removes nothing. If the count is above zero, it sends a short nudge through `client.session.promptAsync` telling the model to run `agent-msg inbox`. The pop happens inside the model's own `agent-msg inbox` call, whose output the model always sees. |
| Session start | The `session.created` event registers the lead. No delivery point fires before the first tool call, so a backlog is delivered at the first tool call or at the first idle nudge. |

Under `opencode run` the process can exit before the model acts on the nudge. Under `opencode serve` and the TUI the server stays up, and the nudge is acted on.

The `chat.message` hook is not used, because text pushed into the first user message did not reach the session's event stream in testing.

## Subagent detection

The plugin treats a session as a lead only after it confirms that `parentID` is unset. It tracks `session.created` and `session.updated` events, and resolves an unknown session through `client.session.get`. A session that cannot be resolved is treated as not a lead.

When one opencode process runs several lead sessions, delivery follows the most recently active confirmed lead. Each hook rechecks after its `agent-msg` round trip that its session is still the most recent, so two idle sessions do not drain one inbox.

## Install

1. Install `agent-msg`:

   ```bash
   cargo install --git https://github.com/agent-ix/ix-relay agent-msg --locked
   ```

2. Run the lead in a genuine verified root session. `agent-msg whoami --json` must report the canonical `server:workspace:tab` identity.

3. Put the machine's bus URL in `~/.ix/relay/config.toml`, as for the [Claude Code plugin](../claude/README.md). [Setup](../../docs/setup.md) explains how to get the URL.

   ```toml
   url = "redis://<machine>:<password>@10.99.0.1:6380"
   machine = "<machine>"
   ```

4. Install the plugin file. opencode loads every file in a project's `.opencode/plugin/` directory, or in `~/.config/opencode/plugin/` for all projects:

   ```bash
   mkdir -p .opencode/plugin
   ln -s /path/to/ix-relay/plugins/opencode/ix-relay.ts .opencode/plugin/ix-relay.ts
   ```

   Alternatively, reference the file from `opencode.json`:

   ```json
   {
     "plugin": ["/path/to/ix-relay/plugins/opencode/ix-relay.ts"]
   }
   ```

5. Give the lead the bus guidance. `plugins/opencode/AGENTS.md` has the same content as the Claude plugin's `/msg` skill. Reference it from `opencode.json`:

   ```json
   {
     "instructions": ["/path/to/ix-relay/plugins/opencode/AGENTS.md"]
   }
   ```

## Verify

```bash
agent-msg whoami --json   # this session's address
agent-msg who             # registered leads
agent-msg log             # recent bus history
```

## Idle leads

An opencode lead that is fully idle is outside the plugin's reach, because no opencode hook fires without a turn. `relay-watcher` covers that case. It wakes the lead by typing into its Herdr pane with `herdr agent prompt`, as it does for Claude Code and Codex leads.
