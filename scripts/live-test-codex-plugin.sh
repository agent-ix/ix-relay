#!/usr/bin/env bash
# Manual, live end-to-end test for the Codex plugin at plugins/codex
# (AGE-2012). Proves the plugin's hooks actually deliver ix-relay messages
# inside real headless Codex sessions, against a real (throwaway) Redis, and
# that a Codex lead and a Claude lead (plugins/claude, AGE-2011) can exchange
# messages both ways. Prints a PASS/FAIL line per check. Exits nonzero if any
# check fails, including a lost race (a race that isn't provably won is a
# FAIL, never a silent pass). Not part of `make ci`: it starts several real
# Codex and Claude Code sessions and spends real API calls.
#
# Checks:
#   backlog             a message queued for the lead before its session
#                       starts is visible at SessionStart, attributed to the
#                       SessionStart hook specifically (not just "some hook")
#                       via a real hook-call log (every hook invocation is
#                       wrapped and logged, see HOOK_CALL_LOG below), and in
#                       the model's own reply
#   midturn             a message queued only after the first tool call
#                       (command_execution) actually COMPLETES is delivered
#                       by PostToolUse specifically (not SessionStart or
#                       UserPromptSubmit), per the same hook-call log
#   backlog-midturn-separate   backlog's and midturn's markers were delivered
#                       in two different hook-call-log entries, not one
#                       hook call answering for both
#   subagent-no-leak    a message pending while a spawn_agent subagent runs is
#                       NOT delivered to any hook call made from inside that
#                       subagent's own context (agent_id set). Proven against
#                       the real hook-call log, not absence-of-evidence: the
#                       subagent must have made at least one hook call (its
#                       own PostToolUse, agent_id set) while the message was
#                       pending, and none of its calls carried the marker.
#                       Retried up to 3 attempts total if the model never
#                       calls spawn_agent; FAILs (not skips) if it never does
#                       in any attempt
#   subagent-lead-gets-it   the lead still receives that same message on its
#                       own next hook call, once the subagent is done
#   stop-blocks          the Stop hook returns {"decision":"block",...} while
#                       a message is pending, so the turn does not end and a
#                       second turn starts; retried up to 3 attempts total,
#                       since the exact moment "pending" is racy against the
#                       model's own turn timing. A run where every attempt
#                       loses the race is a FAIL.
#   exchange-to-claude   the Codex lead's `agent-msg send` reaches the Claude
#                       lead's OWN headless session (plugins/claude,
#                       AGE-2011) and its model reports it
#   exchange-to-codex    the same in the other direction
#
# Every Codex hook call in this script runs through a thin logging wrapper
# (HOOK_WRAPPER) instead of calling `agent-msg hook codex` directly from
# hooks.json. It execs the real binary unchanged (same stdin, stdout, exit
# code) and additionally appends one JSON line per call to HOOK_CALL_LOG:
# {ts, hook_event_name, session_id, agent_id, stdout}. That log is the
# ground truth this script uses to attribute a delivery to a specific hook
# call (and, for a subagent, to prove a call happened at all) — it doesn't
# depend on reverse-engineering Codex's rollout transcript wire format.
#
# Requires: docker, cargo, a `codex` on PATH authenticated to make API calls
# (this script copies ~/.codex/auth.json into an isolated, throwaway
# CODEX_HOME — it never touches the real one), a `claude` on PATH
# authenticated the same way, jq, python3.
#
# Usage:
#   scripts/live-test-codex-plugin.sh [scratch-dir]
#
# scratch-dir defaults to a fresh mktemp -d. Everything the script runs and
# prints is also written to <scratch-dir>/age-2012-live.log. The isolated
# CODEX_HOME's auth.json copy is deleted on exit; the rest of scratch-dir
# (logs, rollout transcripts, the hook-call log) is left for inspection.
#
# Isolation: CODEX_HOME, HOME, and TMPDIR are all pointed inside scratch-dir
# for every `codex exec` this script runs (see codex_exec below) — HOME so
# Codex never touches the real account's ~/.agents (this also happens to
# skip an account-side startup preamble that otherwise adds ~10-20s and a
# noticeable token cost per session, confirmed empirically), and TMPDIR so
# Codex's own additionalContextLimit-truncation dumps
# (normally /tmp/hook_outputs/<session>/) land in scratch instead of the
# real /tmp.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRATCH="${1:-$(mktemp -d)}"
CONTAINER="ixr-codex-livetest-$$"
MODEL="${CODEX_LIVE_TEST_MODEL:-gpt-5.6-hub}"
REAL_CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"

mkdir -p "$SCRATCH"
LOG="$SCRATCH/age-2012-live.log"
exec > >(tee "$LOG") 2>&1

echo "=== ix-relay Codex plugin live test ==="
echo "repo:    $REPO_ROOT"
echo "scratch: $SCRATCH"
echo "log:     $LOG"
echo "model:   $MODEL"
echo

# ---------------------------------------------------------------------------
# PASS/FAIL bookkeeping and background-process tracking
# ---------------------------------------------------------------------------
declare -a CHECK_NAMES=()
declare -a CHECK_RESULTS=()
FAILED=0

pass() {
  local name="$1" detail="${2:-}"
  CHECK_NAMES+=("$name")
  CHECK_RESULTS+=("PASS")
  echo "PASS: $name${detail:+ — $detail}"
}

fail() {
  local name="$1" detail="${2:-}"
  CHECK_NAMES+=("$name")
  CHECK_RESULTS+=("FAIL")
  FAILED=1
  echo "FAIL: $name${detail:+ — $detail}"
}

declare -a BG_PIDS=()
track_pid() { BG_PIDS+=("$1"); }

CODEX_HOME_SCRATCH="$SCRATCH/codex-home"
WORKDIR="$SCRATCH/workspace"
FAKE_HOME="$SCRATCH/fake-home"
SCRATCH_TMPDIR="$SCRATCH/tmp"
HOOK_CALL_LOG="$SCRATCH/hook-calls.jsonl"
HOOK_WRAPPER="$SCRATCH/hook-wrapper.py"

# Kills `pid` and every descendant of it. `codex_exec` backgrounds a shell
# function, so `$!` is that subshell's PID, not the real `codex` (node
# wrapper) and native-binary child processes it execs — killing only the
# subshell left those running past cleanup on earlier drafts of this script.
kill_tree() {
  local pid="$1" child
  for child in $(pgrep -P "$pid" 2>/dev/null || true); do
    kill_tree "$child"
  done
  kill -9 "$pid" 2>/dev/null || true
}

cleanup() {
  echo
  echo "--- cleanup ---"
  for pid in "${BG_PIDS[@]:-}"; do
    [ -z "$pid" ] && continue
    if kill -0 "$pid" 2>/dev/null; then
      echo "killing leftover pid $pid and its children"
      kill_tree "$pid"
    fi
  done
  # Backstop in case tracking missed one: every codex/claude process this run
  # started carries --cd "$WORKDIR" or --plugin-dir plugins/claude in its argv.
  pkill -9 -f -- "--cd $WORKDIR" >/dev/null 2>&1 || true
  pkill -9 -f -- "--plugin-dir $REPO_ROOT/plugins/claude" >/dev/null 2>&1 || true
  echo "stopping redis container $CONTAINER"
  docker stop "$CONTAINER" >/dev/null 2>&1 || true
  # Never leave the copied credential lying around, even though the rest of
  # scratch (logs, rollout transcripts) is left for inspection.
  rm -f "$CODEX_HOME_SCRATCH/auth.json"
  echo "removed scratch CODEX_HOME auth copy"
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# analyze.py: answers the specific questions this test needs from a Codex
# `codex exec --json` event log, a Codex rollout transcript (the jsonl each
# session writes under $CODEX_HOME/sessions/), and a Claude Code
# `--output-format stream-json --include-hook-events` log.
# ---------------------------------------------------------------------------
ANALYZE="$SCRATCH/analyze.py"
cat > "$ANALYZE" <<'PYEOF'
import glob
import json
import sys


def load_jsonl(path):
    events = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return events


# --- Codex: the `codex exec --json` event stream --------------------------

def codex_thread_id(events):
    for ev in events:
        if ev.get("type") == "thread.started":
            return ev.get("thread_id")
    return None


def codex_assistant_texts(events):
    out = []
    for ev in events:
        if ev.get("type") != "item.completed":
            continue
        item = ev.get("item") or {}
        if item.get("type") == "agent_message":
            out.append(item.get("text", ""))
    return out


def codex_count_turns(events):
    return sum(1 for ev in events if ev.get("type") == "turn.started")


def codex_spawned_agent_ids(events):
    """agent_ids of every subagent spawned via spawn_agent, in call order."""
    ids = []
    for ev in events:
        if ev.get("type") != "item.completed":
            continue
        item = ev.get("item") or {}
        if item.get("type") == "collab_tool_call" and item.get("tool") == "spawn_agent":
            for aid in item.get("receiver_thread_ids") or []:
                if aid not in ids:
                    ids.append(aid)
    return ids


def cmd_codex_assistant_contains(events, args):
    (marker,) = args
    for text in codex_assistant_texts(events):
        if marker in text:
            print(f"found in assistant text: {text!r}")
            return 0
    print(f"not found in assistant text (marker {marker!r})")
    return 1


def cmd_codex_turn_count_at_least(events, args):
    (n,) = args
    n = int(n)
    count = codex_count_turns(events)
    if count >= n:
        print(f"saw {count} turn.started events (>= {n})")
        return 0
    print(f"only saw {count} turn.started events (wanted >= {n})")
    return 1


def cmd_codex_spawned_agent_id(events, args):
    ids = codex_spawned_agent_ids(events)
    if ids:
        print(ids[0])
        return 0
    print("no spawn_agent call found", file=sys.stderr)
    return 1


# --- Codex: a rollout transcript (sessions/**/rollout-*-<thread-id>.jsonl) -

def find_rollout(codex_home, thread_id):
    matches = glob.glob(f"{codex_home}/sessions/**/*{thread_id}*.jsonl", recursive=True)
    return matches[0] if matches else None


def rollout_hook_context_texts(rollout_events):
    """Every hook-injected string in a rollout transcript.

    Two distinct wire shapes, both seen in real captures:
      - SessionStart/PostToolUse/UserPromptSubmit's additionalContext lands as
        a role="developer" message tagged content_item_kinds:
        ["hooks.additional_context"].
      - Stop's block `reason` lands as a role="user" message whose text is
        wrapped in `<hook_prompt hook_run_id="stop:...">...</hook_prompt>`
        (content_item_kinds: ["unknown"] — the tag itself is the signal).
    """
    out = []
    for ev in rollout_events:
        if ev.get("type") != "response_item":
            continue
        payload = ev.get("payload") or {}
        if payload.get("type") != "message":
            continue
        meta = payload.get("internal_chat_message_metadata_passthrough") or {}
        kinds = meta.get("content_item_kinds") or []
        is_additional_context = payload.get("role") == "developer" and "hooks.additional_context" in kinds
        for block in payload.get("content") or []:
            if not isinstance(block, dict) or "text" not in block:
                continue
            text = block["text"]
            is_stop_block = payload.get("role") == "user" and text.startswith("<hook_prompt ")
            if is_additional_context or is_stop_block:
                out.append(text)
    return out


def cmd_rollout_contains(codex_home_and_thread, args):
    codex_home, thread_id = codex_home_and_thread
    (marker,) = args
    path = find_rollout(codex_home, thread_id)
    if path is None:
        print(f"no rollout transcript found for thread {thread_id!r} under {codex_home}")
        return 1
    texts = rollout_hook_context_texts(load_jsonl(path))
    for t in texts:
        if marker in t:
            print(f"found in {path} hook-context message: {t!r}")
            return 0
    print(f"not found in {path} ({len(texts)} hook-context messages, marker {marker!r})")
    return 1


def cmd_rollout_not_contains(codex_home_and_thread, args):
    codex_home, thread_id = codex_home_and_thread
    (marker,) = args
    path = find_rollout(codex_home, thread_id)
    if path is None:
        # A missing transcript proves nothing: we cannot tell "genuinely
        # never leaked" from "never captured any evidence at all". Treat it
        # as a FAIL, not a vacuous pass — a check that can't find its own
        # subject isn't a check.
        print(f"FAIL: no rollout transcript found for thread {thread_id!r} under {codex_home}; cannot prove no leak")
        return 1
    texts = rollout_hook_context_texts(load_jsonl(path))
    for t in texts:
        if marker in t:
            print(f"LEAK: {path} hook-context message carried marker: {t!r}")
            return 1
    print(f"NOLEAK: marker not found in {path} ({len(texts)} hook-context messages)")
    return 0


# --- Hook-call log: one JSON line per hook invocation, written by
# hook-wrapper.py (see the shell below). Ground truth for "which hook
# delivered this" and "did this agent_id make a call at all" — doesn't
# depend on Codex's rollout transcript wire format.

def cmd_hookcall_delivered_by(args):
    """0 if some LEAD call (agent_id absent) with hook_event_name==event
    returned stdout containing marker."""
    path, event_name, marker = args
    calls = load_jsonl(path)
    for i, c in enumerate(calls):
        if c.get("agent_id"):
            continue
        if c.get("hook_event_name") != event_name:
            continue
        if marker in (c.get("stdout") or ""):
            print(f"found: hook-call-log entry {i} (hook_event_name={event_name}) delivered {marker!r}")
            return 0
    print(f"not found: no lead hook call with hook_event_name={event_name!r} delivered {marker!r} "
          f"({len(calls)} entries total)")
    return 1


def cmd_hookcall_separate(args):
    """0 if marker1 and marker2 were each delivered by a DIFFERENT
    hook-call-log entry (not the same hook call answering for both)."""
    path, marker1, marker2 = args
    calls = load_jsonl(path)
    idx1 = idx2 = None
    for i, c in enumerate(calls):
        stdout = c.get("stdout") or ""
        if idx1 is None and marker1 in stdout:
            idx1 = i
        if idx2 is None and marker2 in stdout:
            idx2 = i
    if idx1 is None or idx2 is None:
        print(f"could not find both markers in the hook-call log: {marker1!r} at {idx1}, {marker2!r} at {idx2}")
        return 1
    if idx1 == idx2:
        print(f"FAIL: both {marker1!r} and {marker2!r} were delivered by the same hook-call-log entry {idx1}")
        return 1
    print(f"OK: {marker1!r} delivered at entry {idx1}, {marker2!r} at entry {idx2} (separate hook calls)")
    return 0


def cmd_hookcall_agent_called(args):
    """0 if the hook-call log has at least one entry with agent_id==agent_id
    — proof a subagent actually made a hook call, not absence-of-evidence."""
    path, agent_id = args
    calls = load_jsonl(path)
    for c in calls:
        if c.get("agent_id") == agent_id:
            print(f"hook-call log shows a call from agent_id {agent_id!r} (hook_event_name={c.get('hook_event_name')})")
            return 0
    print(f"no hook-call-log entry has agent_id {agent_id!r} ({len(calls)} entries total); "
          f"cannot prove the subagent made any hook call")
    return 1


def cmd_hookcall_no_leak(args):
    """0 if every hook-call-log entry with agent_id==agent_id exists (i.e.
    the subagent did call the hook) AND none of those calls' stdout carried
    marker."""
    path, agent_id, marker = args
    calls = load_jsonl(path)
    seen_any = False
    for c in calls:
        if c.get("agent_id") != agent_id:
            continue
        seen_any = True
        if marker in (c.get("stdout") or ""):
            print(f"LEAK: agent_id {agent_id!r} hook call returned marker {marker!r}: {c.get('stdout')!r}")
            return 1
    if not seen_any:
        print(f"no hook-call-log entry has agent_id {agent_id!r}; nothing to check")
        return 1
    print(f"NOLEAK: none of agent_id {agent_id!r}'s hook calls carried {marker!r}")
    return 0


# --- Claude Code: the same stream-json shape live-test-claude-plugin.sh uses -

def claude_hook_text(ev):
    if ev.get("type") != "system" or ev.get("subtype") != "hook_response":
        return None
    out = (ev.get("output") or "").strip()
    if not out.startswith("{"):
        return None
    try:
        parsed = json.loads(out)
    except json.JSONDecodeError:
        return None
    spec = parsed.get("hookSpecificOutput")
    if isinstance(spec, dict) and "additionalContext" in spec:
        return spec["additionalContext"]
    if "reason" in parsed:
        return parsed["reason"]
    return None


def claude_assistant_texts(events):
    out = []
    for ev in events:
        if ev.get("type") != "assistant":
            continue
        msg = ev.get("message") or {}
        for block in msg.get("content") or []:
            if isinstance(block, dict) and block.get("type") == "text":
                out.append(block.get("text", ""))
    return out


def cmd_claude_hook_contains(events, args):
    event_name, marker = args
    for i, ev in enumerate(events):
        if ev.get("hook_event") != event_name:
            continue
        text = claude_hook_text(ev)
        if text and marker in text:
            print(f"found in {event_name} hook_response at line {i}: {text!r}")
            return 0
    print(f"not found: no {event_name} hook_response contained {marker!r}")
    return 1


def cmd_claude_assistant_contains(events, args):
    (marker,) = args
    for text in claude_assistant_texts(events):
        if marker in text:
            print(f"found in assistant text: {text!r}")
            return 0
    print(f"not found in assistant text (marker {marker!r})")
    return 1


# Dispatch mode per command:
#   "events"  — first arg is a jsonl path, loaded into `events`; fn(events, rest)
#   "rollout" — first two args are (codex_home, thread_id); fn((codex_home, thread_id), rest)
#   "raw"     — every arg passed through untouched; fn(args) does its own loading
COMMANDS = {
    "codex_assistant_contains": (cmd_codex_assistant_contains, "events"),
    "codex_turn_count_at_least": (cmd_codex_turn_count_at_least, "events"),
    "codex_spawned_agent_id": (cmd_codex_spawned_agent_id, "events"),
    "rollout_contains": (cmd_rollout_contains, "rollout"),
    "rollout_not_contains": (cmd_rollout_not_contains, "rollout"),
    "hookcall_delivered_by": (cmd_hookcall_delivered_by, "raw"),
    "hookcall_separate": (cmd_hookcall_separate, "raw"),
    "hookcall_agent_called": (cmd_hookcall_agent_called, "raw"),
    "hookcall_no_leak": (cmd_hookcall_no_leak, "raw"),
    "claude_hook_contains": (cmd_claude_hook_contains, "events"),
    "claude_assistant_contains": (cmd_claude_assistant_contains, "events"),
}


def main():
    if len(sys.argv) < 2:
        print("usage: analyze.py <command> [args...]", file=sys.stderr)
        return 2
    cmd = sys.argv[1]
    if cmd not in COMMANDS:
        print(f"unknown command {cmd!r}", file=sys.stderr)
        return 2
    fn, mode = COMMANDS[cmd]
    rest = sys.argv[2:]
    if mode == "rollout":
        codex_home, thread_id, *args = rest
        return fn((codex_home, thread_id), args)
    if mode == "raw":
        return fn(rest)
    stream_path, *args = rest
    events = load_jsonl(stream_path)
    return fn(events, args)


if __name__ == "__main__":
    sys.exit(main())
PYEOF

# ---------------------------------------------------------------------------
# Build agent-msg and start a throwaway, randomly-ported Redis
# ---------------------------------------------------------------------------
cd "$REPO_ROOT"

echo "--- build agent-msg ---"
set -x
cargo build -p agent-msg
set +x
BIN_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/debug"
export PATH="$BIN_DIR:$PATH"
agent-msg --version
echo

echo "--- start throwaway redis (random host port, container $CONTAINER) ---"
set -x
docker run -d --rm -p 127.0.0.1::6379 --name "$CONTAINER" redis:7
set +x
REDIS_PORT=$(docker port "$CONTAINER" 6379/tcp | head -n1 | sed 's/.*://')
for _ in $(seq 50); do
  docker exec "$CONTAINER" redis-cli ping >/dev/null 2>&1 && break
  sleep 0.1
done
export IX_RELAY_URL="redis://127.0.0.1:${REDIS_PORT}"
echo "IX_RELAY_URL=$IX_RELAY_URL (port $REDIS_PORT, never 6379)"
echo

# ---------------------------------------------------------------------------
# Isolated, throwaway CODEX_HOME: real auth only, nothing else from the
# user's real ~/.codex. hooks.json points at the *built* agent-msg (on PATH
# above) via this plugin's plugins/codex/hooks.json, merged verbatim since
# Codex has no --plugin-dir-style hook auto-loading.
# ---------------------------------------------------------------------------
echo "--- isolated CODEX_HOME: $CODEX_HOME_SCRATCH ---"
mkdir -p "$CODEX_HOME_SCRATCH"
if [ ! -f "$REAL_CODEX_HOME/auth.json" ]; then
  echo "FATAL: no $REAL_CODEX_HOME/auth.json to copy; is codex logged in?"
  exit 1
fi
cp "$REAL_CODEX_HOME/auth.json" "$CODEX_HOME_SCRATCH/auth.json"
chmod 600 "$CODEX_HOME_SCRATCH/auth.json"

WORKDIR="$SCRATCH/workspace"
mkdir -p "$WORKDIR" "$FAKE_HOME" "$SCRATCH_TMPDIR"

cat > "$CODEX_HOME_SCRATCH/config.toml" <<EOF
model = "$MODEL"
model_reasoning_effort = "low"
approval_policy = "never"
sandbox_mode = "danger-full-access"

[features]
hooks = true
multi_agent = true

[projects."$WORKDIR"]
trust_level = "trusted"
EOF

# --- hook-wrapper.py: every hook call in hooks.json runs through this
# instead of `agent-msg hook codex` directly. It execs the real binary
# unchanged (same stdin/stdout/exit code) and appends one JSON line per
# call to HOOK_CALL_LOG — the ground truth this script's checks use to
# attribute a delivery to a specific hook call, and to prove a subagent
# made a call at all (see AGE-2012 review finding #3/#4).
cat > "$HOOK_WRAPPER" <<'PYEOF'
#!/usr/bin/env python3
import json
import os
import subprocess
import sys
import time

raw = sys.stdin.read()
try:
    parsed = json.loads(raw)
except json.JSONDecodeError:
    parsed = {}

agent_msg_bin = os.environ.get("AGENT_MSG_BIN", "agent-msg")
proc = subprocess.run(
    [agent_msg_bin, "hook", "codex"],
    input=raw,
    capture_output=True,
    text=True,
)
sys.stdout.write(proc.stdout)
sys.stderr.write(proc.stderr)

log_path = os.environ.get("HOOK_CALL_LOG")
if log_path:
    entry = {
        "ts": time.time(),
        "hook_event_name": parsed.get("hook_event_name"),
        "session_id": parsed.get("session_id"),
        "agent_id": parsed.get("agent_id"),
        "stdout": proc.stdout,
    }
    with open(log_path, "a") as f:
        f.write(json.dumps(entry) + "\n")

sys.exit(proc.returncode)
PYEOF
chmod +x "$HOOK_WRAPPER"
: > "$HOOK_CALL_LOG"

python3 -c "
import json
with open('$REPO_ROOT/plugins/codex/hooks.json') as f:
    cfg = json.load(f)
for hooks_for_event in cfg['hooks'].values():
    for matcher in hooks_for_event:
        for h in matcher['hooks']:
            if h.get('command') == 'agent-msg hook codex':
                h['command'] = '$HOOK_WRAPPER'
with open('$CODEX_HOME_SCRATCH/hooks.json', 'w') as f:
    json.dump(cfg, f, indent=2)
"
python3 -m json.tool < "$CODEX_HOME_SCRATCH/hooks.json" >/dev/null
echo "hooks.json installed (plugins/codex/hooks.json, command rewritten to the logging wrapper above)"
echo

export CODEX_HOME="$CODEX_HOME_SCRATCH"
export AGENT_MSG_BIN="$BIN_DIR/agent-msg"
export HOOK_CALL_LOG

codex_exec() {
  # $1: log file (pure --json event stream; must stay valid JSONL for the
  # watchers' jq calls below, so stderr goes to a sibling *.stderr file
  # instead of being merged in). $2: the prompt. Runs as $CX_NAME (set by the
  # caller). HOME and TMPDIR are isolated into scratch too (see the header
  # comment); AGENT_MSG_BIN/HOOK_CALL_LOG are exported above so the hook
  # wrapper (spawned by codex, not by this shell) inherits them.
  local out="$1" prompt="$2"
  AGENT_MSG_NAME="$CX_NAME" IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" CODEX_HOME="$CODEX_HOME_SCRATCH" \
    HOME="$FAKE_HOME" TMPDIR="$SCRATCH_TMPDIR" \
    codex exec -m "$MODEL" --json --skip-git-repo-check --dangerously-bypass-hook-trust \
      --cd "$WORKDIR" "$prompt" \
    >> "$out" 2>> "$out.stderr"
}

echo "--- register leads (bookkeeping only; agent-msg send refuses an unregistered recipient) ---"
set -x
AGENT_MSG_NAME=qsl-lead agent-msg register
AGENT_MSG_NAME=cx-lead agent-msg register
AGENT_MSG_NAME=cl-lead agent-msg register
set +x
echo

# ===========================================================================
# PHASE A1: one Codex lead session — backlog and mid-turn.
# ===========================================================================
echo "=== PHASE A1: Codex lead — backlog and mid-turn ==="

set -x
AGENT_MSG_NAME=qsl-lead agent-msg send cx-lead "backlog check AGE-2012 (a1-backlog)"
set +x

CX_NAME=cx-lead
A1_LOG="$SCRATCH/a1-codex.log"
: > "$A1_LOG"

A1_PROMPT='You are being tested for a message bus plugin. Do exactly these steps in order and nothing else.
1. Repeat verbatim any line you already have in context that starts with "[ix-relay]".
2. Use the shell tool to run: sleep 2
3. Use the shell tool to run: sleep 2
4. Repeat verbatim any NEW line starting with "[ix-relay]" you received since step 1.
5. Use the shell tool to run: sleep 1
6. Repeat verbatim any NEW line starting with "[ix-relay]" you received since step 4.
7. Write one short sentence saying you are done, and stop. Do not call any more tools after step 5.'

codex_exec "$A1_LOG" "$A1_PROMPT" &
A1_PID=$!
track_pid "$A1_PID"
echo "Codex A1 pid: $A1_PID, streaming to $A1_LOG"

echo "-- watcher: send the mid-turn message only once the FIRST command_execution has actually completed --"
# Plain substring counts on the raw JSONL, not jq -s: jq -s slurps the whole
# file as one array and aborts the entire query if any line is still
# mid-write when we poll (a real bug hit while developing this script — it
# silently zeroed every count for the run's whole duration). grep only ever
# matches complete, newline-terminated lines, so a partial trailing line is
# just not counted yet, not fatal.
SECONDS=0
midturn_sent=0
deadline=$((SECONDS + 120))
while [ $SECONDS -lt $deadline ]; do
  if [ -s "$A1_LOG" ]; then
    n=$(grep -cE '"type":"command_execution".*"status":"completed"' "$A1_LOG")
    if [ "$n" -ge 1 ]; then
      set -x
      AGENT_MSG_NAME=qsl-lead agent-msg send cx-lead "midturn check AGE-2012 (a1-midturn)"
      set +x
      midturn_sent=1
      break
    fi
  fi
  sleep 0.1
done
[ "$midturn_sent" -eq 1 ] || echo "WARNING: never saw a completed command_execution within 120s (shouldn't happen)"

wait "$A1_PID" || true
echo "-- A1 session finished --"
echo

echo "-- A1 checks (against the hook-call log, not the rollout transcript) --"
if python3 "$ANALYZE" hookcall_delivered_by "$HOOK_CALL_LOG" SessionStart "a1-backlog" \
   && python3 "$ANALYZE" codex_assistant_contains "$A1_LOG" "a1-backlog"; then
  pass "backlog" "SessionStart hook call delivered it and the model repeated it"
else
  fail "backlog" "see analyze.py output above"
fi

if python3 "$ANALYZE" hookcall_delivered_by "$HOOK_CALL_LOG" PostToolUse "a1-midturn" \
   && python3 "$ANALYZE" codex_assistant_contains "$A1_LOG" "a1-midturn"; then
  pass "midturn" "PostToolUse hook call (sent only after the first command_execution completed) delivered it and the model repeated it"
else
  fail "midturn" "see analyze.py output above"
fi

if python3 "$ANALYZE" hookcall_separate "$HOOK_CALL_LOG" "a1-backlog" "a1-midturn"; then
  pass "backlog-midturn-separate" "backlog and midturn were delivered by two different hook calls"
else
  fail "backlog-midturn-separate" "see analyze.py output above"
fi
echo

# ===========================================================================
# PHASE A1B: a fresh Codex lead session, spawning exactly one subagent —
# proves a message pending while the subagent runs is never delivered to the
# subagent's own hook calls, using real evidence (the hook-call log), then
# that the lead itself gets it once the subagent is done. Racy (the window
# is "between spawn_agent returning and the subagent's own first tool call"),
# and the model isn't guaranteed to call spawn_agent at all, so retried up to
# 3 attempts total; a run where every attempt fails is a FAIL, never a SKIP.
# ===========================================================================
echo "=== PHASE A1B: Codex subagent hook silence (up to 3 attempts) ==="

SUBAGENT_PROVEN=0
for attempt in 1 2 3; do
  echo "-- attempt $attempt --"
  marker="a1b-subagent-attempt${attempt}"
  A1B_LOG="$SCRATCH/a1b-attempt${attempt}.log"
  : > "$A1B_LOG"

  A1B_PROMPT="You are being tested for a message bus plugin. Do exactly these steps in order and nothing else.
1. Call your collaboration.spawn_agent tool (or equivalent subagent-spawning tool) to spawn exactly one subagent with the instruction: \"Use the shell tool to run: sleep 3. Then reply that you are done.\"
2. Use collaboration.wait_agent (or equivalent) to wait for it to finish.
3. Use the shell tool to run: sleep 1
4. Repeat verbatim any NEW line starting with \"[ix-relay]\" you received since step 1.
5. Write one short sentence saying you are done, and stop."

  codex_exec "$A1B_LOG" "$A1B_PROMPT" &
  A1B_PID=$!
  track_pid "$A1B_PID"
  echo "attempt $attempt pid: $A1B_PID"

  spawn_sent=0
  deadline=$((SECONDS + 90))
  while [ $SECONDS -lt $deadline ]; do
    if [ -s "$A1B_LOG" ] && [ "$(grep -c '"receiver_thread_ids":\["' "$A1B_LOG")" -ge 1 ]; then
      set -x
      AGENT_MSG_NAME=qsl-lead agent-msg send cx-lead "subagent window check AGE-2012 (${marker})"
      set +x
      spawn_sent=1
      break
    fi
    sleep 0.1
  done
  [ "$spawn_sent" -eq 1 ] || echo "attempt $attempt: never saw a spawn_agent call within 90s"

  wait "$A1B_PID" || true

  if [ "$spawn_sent" -eq 1 ]; then
    SUB_AGENT_ID=$(python3 "$ANALYZE" codex_spawned_agent_id "$A1B_LOG" 2>/dev/null)
    if [ -n "$SUB_AGENT_ID" ] \
       && python3 "$ANALYZE" hookcall_agent_called "$HOOK_CALL_LOG" "$SUB_AGENT_ID" \
       && python3 "$ANALYZE" hookcall_no_leak "$HOOK_CALL_LOG" "$SUB_AGENT_ID" "$marker" \
       && python3 "$ANALYZE" rollout_not_contains "$CODEX_HOME_SCRATCH" "$SUB_AGENT_ID" "$marker"; then
      pass "subagent-no-leak" "subagent (agent_id=$SUB_AGENT_ID) made >=1 hook call while ${marker} was pending; none carried it"
      if python3 "$ANALYZE" hookcall_delivered_by "$HOOK_CALL_LOG" PostToolUse "$marker"; then
        pass "subagent-lead-gets-it" "the lead's own next PostToolUse hook call delivered ${marker} once the subagent finished"
      else
        fail "subagent-lead-gets-it" "see analyze.py output above"
      fi
      SUBAGENT_PROVEN=1
      break
    fi
    echo "attempt $attempt: spawn happened but the hook-call-log proof was incomplete; see analyze.py output above"
  fi
done

if [ "$SUBAGENT_PROVEN" -eq 0 ]; then
  fail "subagent-no-leak" "no attempt (of 3) produced a spawn_agent call with provable hook-call-log evidence"
  fail "subagent-lead-gets-it" "no attempt (of 3) produced a spawn_agent call with provable hook-call-log evidence"
fi
echo

# ===========================================================================
# PHASE A2: the Stop hook blocks the turn while a message is pending, seen as
# a second turn.started in the same `codex exec` invocation. Racy by nature
# (it depends on landing the send between the last PostToolUse and Stop), so
# retried up to 3 attempts. A run where every attempt loses the race is a
# FAIL, not a silent pass.
# ===========================================================================
echo "=== PHASE A2: Stop hook blocks while a message is pending (up to 3 attempts) ==="

STOP_WON=0
for attempt in 1 2 3; do
  echo "-- attempt $attempt --"
  marker="a2-stopblock-attempt${attempt}"
  A2_LOG="$SCRATCH/a2-attempt${attempt}.log"
  : > "$A2_LOG"
  A2_PROMPT="Do exactly these steps in order and nothing else.
1. Use the shell tool to run: sleep 1
2. Use the shell tool to run: sleep 1
3. Write one short sentence saying you are done, and stop. Do not call any more tools after step 2."

  codex_exec "$A2_LOG" "$A2_PROMPT" &
  A2_PID=$!
  track_pid "$A2_PID"
  echo "attempt $attempt pid: $A2_PID"

  sent=0
  deadline=$((SECONDS + 60))
  while [ $SECONDS -lt $deadline ]; do
    if [ -s "$A2_LOG" ]; then
      # Substring count, not jq -s: see the comment above phase A1's watcher.
      n=$(grep -cE '"type":"command_execution".*"status":"completed"' "$A2_LOG")
      if [ "$n" -ge 2 ]; then
        set -x
        AGENT_MSG_NAME=qsl-lead agent-msg send cx-lead "stop-block check AGE-2012 (${marker})"
        set +x
        sent=1
        break
      fi
    fi
    sleep 0.05
  done
  [ "$sent" -eq 1 ] || echo "WARNING: never saw 2 completed command_executions within 60s on attempt $attempt"

  wait "$A2_PID" || true

  if [ "$sent" -eq 1 ]; then
    A2_THREAD=$(jq -s '[.[] | select(.type=="thread.started")][0].thread_id' -r "$A2_LOG" 2>/dev/null)
    # A real block doesn't necessarily add a second top-level turn.started in
    # `codex exec`'s event stream (a block continues the same turn instead of
    # starting a distinguishable new one there) — the reliable signal is the
    # rollout transcript itself: a role="user" message wrapped in
    # `<hook_prompt hook_run_id="stop:...">` carrying our marker, which only
    # exists if Stop's `{"decision":"block","reason":...}` was honored.
    if python3 "$ANALYZE" rollout_contains "$CODEX_HOME_SCRATCH" "$A2_THREAD" "$marker"; then
      echo "attempt $attempt: WON the race (Stop hook's block reason carried our marker)"
      STOP_WON=1
      break
    fi
  fi
  echo "attempt $attempt: LOST the race"
done

if [ "$STOP_WON" -eq 1 ]; then
  pass "stop-blocks" "Stop hook forced a second turn while our marker was the pending message"
else
  fail "stop-blocks" "all 3 attempts lost the race or failed to send in time"
fi
echo

# ===========================================================================
# PHASE B: a live Codex lead and a live Claude lead (plugins/claude,
# AGE-2011) exchange messages in both directions over the same Redis.
# ===========================================================================
echo "=== PHASE B: Codex lead and Claude lead exchange messages in both directions ==="

# Codex's own account-side preamble (see plugins/codex/README.md / the PR
# description's Q&A) adds 10-20s of extra tool calls before either side even
# reaches step 1, and it isn't symmetric between the two harnesses, so both
# sides get generous wait budgets rather than tight ones.
CX_PROMPT='You are cx-lead, being tested for a message bus plugin. Do exactly these steps in order and nothing else.
1. Use the shell tool to run: sleep 2
2. Use the shell tool to run: agent-msg send cl-lead "hello from cx-lead exchange test"
3. Use the shell tool to run: sleep 30
4. Repeat verbatim any line starting with "[ix-relay]" that you have received.
5. Write one short sentence saying you are done, and stop.'

CL_PROMPT='You are cl-lead, being tested for a message bus plugin. Do exactly these steps in order and nothing else.
1. Use the Bash tool to run: sleep 1
2. Use the Bash tool to run: sleep 15
3. Repeat verbatim any line starting with "[ix-relay]" that you have received.
4. Use the Bash tool to run: agent-msg send cx-lead "hello back from cl-lead exchange test"
5. Write one short sentence saying you are done, and stop.'

CX_STREAM="$SCRATCH/exchange-cx.log"
CL_STREAM="$SCRATCH/exchange-cl.log"
: > "$CX_STREAM"
: > "$CL_STREAM"

CX_NAME=cx-lead
codex_exec "$CX_STREAM" "$CX_PROMPT" &
CX_PID=$!
track_pid "$CX_PID"

AGENT_MSG_NAME=cl-lead IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
  claude -p --model haiku \
    --plugin-dir "$REPO_ROOT/plugins/claude" \
    --permission-mode bypassPermissions \
    --setting-sources project,local \
    --output-format stream-json --include-hook-events --verbose \
    "$CL_PROMPT" \
  >> "$CL_STREAM" 2> "$SCRATCH/exchange-cl-stderr.txt" &
CL_PID=$!
track_pid "$CL_PID"

echo "cx-lead (Codex) pid: $CX_PID, cl-lead (Claude) pid: $CL_PID, running concurrently"
wait "$CX_PID" || true
wait "$CL_PID" || true
echo "-- both exchange sessions finished --"
echo

echo "-- exchange checks --"
if python3 "$ANALYZE" claude_hook_contains "$CL_STREAM" PostToolUse "hello from cx-lead exchange test" \
   && python3 "$ANALYZE" claude_assistant_contains "$CL_STREAM" "hello from cx-lead exchange test"; then
  pass "exchange-to-claude" "cl-lead's own live Claude Code session received and reported cx-lead's message"
else
  fail "exchange-to-claude" "see analyze.py output above"
fi

CX_THREAD=$(jq -s '[.[] | select(.type=="thread.started")][0].thread_id' -r "$CX_STREAM" 2>/dev/null)
if python3 "$ANALYZE" rollout_contains "$CODEX_HOME_SCRATCH" "$CX_THREAD" "hello back from cl-lead exchange test" \
   && python3 "$ANALYZE" codex_assistant_contains "$CX_STREAM" "hello back from cl-lead exchange test"; then
  pass "exchange-to-codex" "cx-lead's own live Codex session received and reported cl-lead's reply"
else
  fail "exchange-to-codex" "see analyze.py output above"
fi
echo

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo "=== summary ==="
for i in "${!CHECK_NAMES[@]}"; do
  echo "${CHECK_RESULTS[$i]}: ${CHECK_NAMES[$i]}"
done
echo

if [ "$FAILED" -eq 0 ]; then
  echo "=== all checks passed ==="
else
  echo "=== at least one check FAILED ==="
fi
exit "$FAILED"
