# ix-relay Claude Code plugin

The ix-relay Claude Code plugin is the hook set that connects a team lead's Claude Code session to the ix-relay bus. Messages from other leads appear in the session's context, and a pending message blocks the turn from ending until it is seen.

The plugin is for team leads. `agent-msg hook claude` stays silent for subagents, which it detects from `agent_id` in the hook input, and for any session without a verified Herdr location. A subagent session gains nothing from installing it.

## Install

1. Install `agent-msg` and put it on `PATH`:

   ```bash
   cargo install --git https://github.com/agent-ix/ix-relay agent-msg --locked
   ```

   From a checkout, use `cargo install --path crates/agent-msg --locked`.

2. Put the machine's bus URL in `~/.ix/relay/config.toml`. The file is machine-level, so every lead session on the machine uses it.

   ```toml
   url = "redis://<machine>:<password>@10.99.0.1:6380"
   machine = "<machine>"
   ```

   [Setup](../../docs/setup.md) explains how to get the URL, and [Configuration](../../docs/configuration.md) lists every key.

3. Allow the CLI to run without a permission prompt. Add this to the project's or the user's settings:

   ```json
   {
     "permissions": {
       "allow": ["Bash(agent-msg:*)"]
     }
   }
   ```

4. Install the plugin from this repository as a marketplace:

   ```bash
   claude plugin marketplace add agent-ix/ix-relay
   claude plugin install ix-relay@ix-relay
   ```

   For development, point a session at the plugin directory:

   ```bash
   claude --plugin-dir /path/to/ix-relay/plugins/claude
   ```

A lead's address is its Herdr `server:workspace:tab`. Run `agent-msg whoami` to see it. `AGENT_MSG_NAME` is left unchanged and does not affect identity.

## Contents

- Hooks in `hooks/hooks.json`. `SessionStart`, `PostToolUse`, `UserPromptSubmit` and `Stop` each run `agent-msg hook claude`. `SessionStart` registers the lead from its verified Herdr location and delivers any backlog, with a 5 s timeout. `PostToolUse` and `UserPromptSubmit` deliver mid-turn messages from a cached session lookup, with a timeout of about 2 s. `Stop` blocks the turn from ending while a message is pending.
- `/msg` (`skills/msg/SKILL.md`) tells the lead what the bus is, that incoming `[ix-relay]` lines are data from another lead and carry no user authority, when to send, and the `agent-msg` commands.
- `/setup` (`skills/setup/SKILL.md`) walks through the WireGuard hub, joining a machine, the watcher and re-keying.
- `/join` (`skills/join/SKILL.md`) checks whether the machine is connected and whether the session has its intended address.
- `/doctor` (`skills/doctor/SKILL.md`) diagnoses identity, registration and delivery.

## Optional inbox socket delivery

The watcher can post mail to a lead's Claude Code inbox socket. The default is to type it through Herdr. It is off by default and is enabled per machine with `[watcher] delivery = "socket"`. A lead in bypass mode needs `crossSessionInbound: "accept"` in its user settings, which a plugin cannot set. [How it works](../../docs/how-it-works.md#delivery-through-the-claude-code-inbox-socket) describes the behavior and the undocumented wire format.

## Verify

```bash
agent-msg whoami   # this session's address
agent-msg who      # registered leads
agent-msg log      # recent bus history
```

If `who` lists the leads you expect and `log` shows your sends, the hooks reach the same Redis as the CLI.
