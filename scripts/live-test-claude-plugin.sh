#!/usr/bin/env bash
# Manual, live end-to-end test for the Claude Code plugin at plugins/claude
# (AGE-2011). Proves the plugin's hooks actually deliver ix-relay messages
# inside real headless Claude Code sessions, against a real (throwaway)
# Redis, and prints a PASS/FAIL line per check. Exits nonzero if any check
# fails, including a lost race (a race that isn't provably won is a FAIL,
# never a silent pass). Not part of `make ci`: it starts several real Claude
# Code sessions and spends real API calls.
#
# Checks:
#   backlog          a message queued for a lead before its session starts
#                     is visible at SessionStart (both in the hook JSON and
#                     in the model's own reply)
#   midturn           a message queued mid-turn is delivered by PostToolUse
#   subagent-no-leak  a message pending while a spawned subagent runs is
#                     NOT delivered to any hook call made from inside that
#                     subagent (agent_id set)
#   subagent-lead-gets-it   the lead still receives that same message on its
#                     own next hook call, once the subagent is done
#   stop-blocks        the Stop hook returns {"decision":"block",...} while a
#                     message is pending, so the turn does not end; retried
#                     up to 3 attempts total, since the exact moment "pending"
#                     is racy against the model's own turn timing. A run
#                     where every attempt loses the race is a FAIL.
#   exchange-to-qsl    tl-lead's `agent-msg send` reaches qsl-lead's OWN
#                     headless session (not the CLI) and qsl-lead's model
#                     reports it
#   exchange-to-tl     the same in the other direction
#
# Requires: docker, cargo, a `claude` on PATH authenticated to make API
# calls, jq, python3.
#
# Usage:
#   scripts/live-test-claude-plugin.sh [scratch-dir]
#
# scratch-dir defaults to a fresh mktemp -d. Everything the script runs and
# prints is also written to <scratch-dir>/age-2011-live.log.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRATCH="${1:-$(mktemp -d)}"
CONTAINER="ixr-plugin-livetest-$$"

mkdir -p "$SCRATCH"
LOG="$SCRATCH/age-2011-live.log"
exec > >(tee "$LOG") 2>&1

echo "=== ix-relay Claude Code plugin live test ==="
echo "repo:    $REPO_ROOT"
echo "scratch: $SCRATCH"
echo "log:     $LOG"
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

cleanup() {
  echo
  echo "--- cleanup ---"
  for pid in "${BG_PIDS[@]:-}"; do
    [ -z "$pid" ] && continue
    if kill -0 "$pid" 2>/dev/null; then
      echo "killing leftover claude pid $pid"
      kill -9 "$pid" 2>/dev/null || true
    fi
  done
  echo "stopping redis container $CONTAINER"
  docker stop "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# analyze.py: parses a claude --output-format stream-json --include-hook-events
# log and answers the specific questions this test needs. Written to scratch
# at run time so this one script file is what's committed.
# ---------------------------------------------------------------------------
ANALYZE="$SCRATCH/analyze.py"
cat > "$ANALYZE" <<'PYEOF'
import json
import sys


def load(path):
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


def hook_text(ev):
    """Extract the text ix-relay put in a hook_response's output, if any."""
    if ev.get("type") != "system" or ev.get("subtype") != "hook_response":
        return None
    out = ev.get("output") or ""
    out = out.strip()
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


def assistant_texts(events):
    out = []
    for ev in events:
        if ev.get("type") != "assistant":
            continue
        msg = ev.get("message") or {}
        for block in msg.get("content") or []:
            if isinstance(block, dict) and block.get("type") == "text":
                out.append(block.get("text", ""))
    return out


def cmd_hook_contains(events, args):
    event_name, marker = args
    hits = []
    for i, ev in enumerate(events):
        if ev.get("hook_event") != event_name:
            continue
        text = hook_text(ev)
        if text and marker in text:
            hits.append((i, text))
    if hits:
        print(f"found in {event_name} hook_response at line {hits[0][0]}: {hits[0][1]!r}")
        return 0
    print(f"not found: no {event_name} hook_response contained {marker!r}")
    return 1


def cmd_assistant_contains(events, args):
    (marker,) = args
    for text in assistant_texts(events):
        if marker in text:
            print(f"found in assistant text: {text!r}")
            return 0
    print(f"not found in assistant text (marker {marker!r})")
    return 1


def cmd_stop_block_first(events, args):
    (marker,) = args
    stops = [(i, ev) for i, ev in enumerate(events) if ev.get("hook_event") == "Stop" and ev.get("subtype") == "hook_response"]
    if not stops:
        print("no Stop hook_response events at all")
        return 1
    i, first = stops[0]
    out = (first.get("output") or "").strip()
    if not out.startswith("{"):
        print(f"first Stop hook_response (line {i}) had no JSON output: {out!r}")
        return 1
    try:
        parsed = json.loads(out)
    except json.JSONDecodeError:
        print(f"first Stop hook_response (line {i}) had unparseable output: {out!r}")
        return 1
    if parsed.get("decision") == "block" and marker in parsed.get("reason", ""):
        print(f"first Stop hook_response (line {i}) blocked with reason: {parsed['reason']!r}")
        return 0
    print(f"first Stop hook_response (line {i}) did not block on our marker: {parsed!r}")
    return 1


def cmd_subagent_silence(events, args):
    (marker,) = args
    # A plain "sleep 5" also gets a task_started/task_notification pair
    # (task_type "local_bash"). The actual subagent spawn is the one with a
    # subagent_type (task_type "local_agent") — that's the one we need to
    # bracket, not just the first task_started of any kind.
    task_started = None
    for i, ev in enumerate(events):
        if ev.get("subtype") == "task_started" and ev.get("subagent_type"):
            task_started = (i, ev)
            break
    if task_started is None:
        print("no subagent task_started event found; subagent never spawned")
        return 1
    ts_idx, ts_ev = task_started
    tuid = ts_ev.get("tool_use_id")
    task_id = ts_ev.get("task_id")

    subagent_start = None
    for i in range(ts_idx + 1, len(events)):
        ev = events[i]
        if ev.get("type") in ("assistant", "user") and ev.get("parent_tool_use_id") == tuid:
            subagent_start = i
            break
    if subagent_start is None:
        print(f"never saw a message with parent_tool_use_id={tuid!r}; can't bracket the subagent's own work")
        return 1

    task_end = len(events)
    for i in range(ts_idx + 1, len(events)):
        ev = events[i]
        if ev.get("subtype") == "task_notification" and ev.get("task_id") == task_id:
            task_end = i
            break

    leaked_at = None
    for i in range(subagent_start, task_end):
        text = hook_text(events[i])
        if text and marker in text:
            leaked_at = i
            break
    if leaked_at is not None:
        print(f"LEAK: hook_response inside the subagent's own window (line {leaked_at}) carried our marker")
        return 1

    got_it_at = None
    for i, ev in enumerate(events):
        if subagent_start <= i < task_end:
            continue
        text = hook_text(ev)
        if text and marker in text:
            got_it_at = i
            break
    if got_it_at is None:
        print(f"MISSED: marker never showed up in any hook_response outside the subagent window "
              f"(subagent window was lines {subagent_start}..{task_end})")
        return 1

    print(f"NOLEAK inside subagent window (lines {subagent_start}..{task_end}); "
          f"lead got it at line {got_it_at} via its own hook")
    return 0


COMMANDS = {
    "hook_contains": cmd_hook_contains,
    "assistant_contains": cmd_assistant_contains,
    "stop_block_first": cmd_stop_block_first,
    "subagent_silence": cmd_subagent_silence,
}


def main():
    if len(sys.argv) < 3:
        print("usage: analyze.py <stream.jsonl> <command> [args...]", file=sys.stderr)
        return 2
    stream_path, cmd = sys.argv[1], sys.argv[2]
    args = sys.argv[3:]
    if cmd not in COMMANDS:
        print(f"unknown command {cmd!r}", file=sys.stderr)
        return 2
    events = load(stream_path)
    return COMMANDS[cmd](events, args)


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
echo "IX_RELAY_URL=$IX_RELAY_URL"
echo

# Isolate headless sessions from the runner's own user-level settings/plugins
# (e.g. an unrelated always-on output-style plugin) while keeping --plugin-dir
# working: --setting-sources without "user" still honors --plugin-dir, which
# loads for the session regardless of settings sources.
SETTING_SOURCES="project,local"

cd "$SCRATCH"

echo "--- register both leads (bookkeeping only; agent-msg send refuses an unregistered recipient) ---"
set -x
AGENT_MSG_NAME=qsl-lead agent-msg register
AGENT_MSG_NAME=tl-lead agent-msg register
set +x
echo

# ===========================================================================
# PHASE A1: tl-lead session — backlog delivery, mid-turn delivery, and
# subagent silence, all in one session.
# ===========================================================================
echo "=== PHASE A1: tl-lead — backlog, mid-turn, subagent silence ==="

set -x
AGENT_MSG_NAME=qsl-lead agent-msg send tl-lead "backlog check AGE-2011 (a1-backlog)"
set +x

A1_PROMPT='You are being tested for a message bus plugin. Do exactly these steps in order and nothing else.
1. Repeat verbatim any line you already have in context that starts with "[ix-relay]".
2. Use the Bash tool to run: sleep 5
3. Use the Bash tool to run: sleep 5
4. Repeat verbatim any NEW line starting with "[ix-relay]" you received since step 1.
5. Use the Task/Agent tool to spawn exactly one general-purpose subagent with this exact instruction: "Use the Bash tool to run: sleep 5. Then reply that you are done." Wait for it to finish.
6. Use the Bash tool to run: sleep 1
7. Repeat verbatim any NEW line starting with "[ix-relay]" you received since step 4.
8. Write one short sentence saying you are done, and stop. Do not call any more tools after step 6.'

A1_STREAM="$SCRATCH/a1-stream.jsonl"
: > "$A1_STREAM"

AGENT_MSG_NAME=tl-lead IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
  claude -p --model haiku \
    --plugin-dir "$REPO_ROOT/plugins/claude" \
    --permission-mode bypassPermissions \
    --setting-sources "$SETTING_SOURCES" \
    --output-format stream-json --include-hook-events --verbose \
    "$A1_PROMPT" \
  >> "$A1_STREAM" 2> "$SCRATCH/a1-stderr.txt" &
A1_PID=$!
track_pid "$A1_PID"
echo "tl-lead A1 pid: $A1_PID, streaming to $A1_STREAM"

echo "-- watcher: mid-turn message once UserPromptSubmit's own hook has already run (so it lands in the first sleep, not that hook); subagent-window message when the subagent's own task_started appears --"
SECONDS=0
midturn_sent=0
subagent_sent=0
deadline=$((SECONDS + 90))
while [ $SECONDS -lt $deadline ]; do
  if [ "$midturn_sent" -eq 0 ] && [ -s "$A1_STREAM" ]; then
    # Wait for UserPromptSubmit's hook_response to show up before sending: this
    # plugin also hooks UserPromptSubmit, and it fires very early (before the
    # model calls any tool). Sending too early risks the message being
    # delivered there instead of by PostToolUse, which is what this check
    # needs to prove.
    upsubmit_done=$(jq -s '[.[] | select(.type=="system" and .subtype=="hook_response" and .hook_event=="UserPromptSubmit")] | length' "$A1_STREAM" 2>/dev/null || echo 0)
    if [ "$upsubmit_done" -ge 1 ]; then
      set -x
      AGENT_MSG_NAME=qsl-lead agent-msg send tl-lead "midturn check AGE-2011 (a1-midturn)"
      set +x
      midturn_sent=1
    fi
  fi
  if [ "$subagent_sent" -eq 0 ] && [ -s "$A1_STREAM" ]; then
    # Filter for the subagent's own task_started (subagent_type set), not a
    # plain "sleep 5" task_started (also task_started, but task_type local_bash).
    has_task=$(jq -s '[.[] | select(.subtype=="task_started" and (.subagent_type != null))] | length' "$A1_STREAM" 2>/dev/null || echo 0)
    if [ "$has_task" -ge 1 ]; then
      set -x
      AGENT_MSG_NAME=qsl-lead agent-msg send tl-lead "subagent window check AGE-2011 (a1-subagent)"
      set +x
      subagent_sent=1
    fi
  fi
  if [ "$midturn_sent" -eq 1 ] && [ "$subagent_sent" -eq 1 ]; then
    break
  fi
  sleep 0.2
done
[ "$midturn_sent" -eq 1 ] || echo "WARNING: never reached t=2s watcher tick (shouldn't happen)"
[ "$subagent_sent" -eq 1 ] || echo "WARNING: never saw task_started within 90s; subagent message not sent"

wait "$A1_PID" || true
echo "-- A1 session finished --"
echo

echo "-- A1 checks --"
if python3 "$ANALYZE" "$A1_STREAM" hook_contains SessionStart "a1-backlog" \
   && python3 "$ANALYZE" "$A1_STREAM" assistant_contains "a1-backlog"; then
  pass "backlog" "SessionStart hook delivered it and tl-lead repeated it"
else
  fail "backlog" "see analyze.py output above"
fi

if python3 "$ANALYZE" "$A1_STREAM" hook_contains PostToolUse "a1-midturn" \
   && python3 "$ANALYZE" "$A1_STREAM" assistant_contains "a1-midturn"; then
  pass "midturn" "PostToolUse hook delivered it and tl-lead repeated it"
else
  fail "midturn" "see analyze.py output above"
fi

if [ "$subagent_sent" -eq 1 ]; then
  if python3 "$ANALYZE" "$A1_STREAM" subagent_silence "a1-subagent"; then
    pass "subagent-no-leak" "no hook_response inside the subagent's own window carried the marker"
    pass "subagent-lead-gets-it" "tl-lead's own hook delivered it once the subagent finished"
  else
    fail "subagent-no-leak" "see analyze.py output above (also fails subagent-lead-gets-it)"
    fail "subagent-lead-gets-it" "see analyze.py output above"
  fi
else
  fail "subagent-no-leak" "message was never sent (watcher never saw task_started)"
  fail "subagent-lead-gets-it" "message was never sent (watcher never saw task_started)"
fi
echo

# ===========================================================================
# PHASE A2: the Stop hook blocks the turn while a message is pending.
# Racy by nature (it depends on landing the send between the last PostToolUse
# and Stop), so retried up to 3 attempts. A run where every attempt loses the
# race is a FAIL, not a silent pass.
# ===========================================================================
echo "=== PHASE A2: Stop hook blocks while a message is pending (up to 3 attempts) ==="

STOP_WON=0
for attempt in 1 2 3; do
  echo "-- attempt $attempt --"
  marker="a2-stopblock-attempt${attempt}"
  A2_STREAM="$SCRATCH/a2-attempt${attempt}-stream.jsonl"
  : > "$A2_STREAM"
  A2_PROMPT="You are being tested. Do exactly these steps in order and nothing else.
1. Use the Bash tool to run: sleep 2
2. Use the Bash tool to run: agent-msg send qsl-lead \"ack attempt ${attempt}\"
3. Write one short sentence saying you are done, and stop. Do not call any more tools after step 2."

  AGENT_MSG_NAME=tl-lead IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
    claude -p --model haiku \
      --plugin-dir "$REPO_ROOT/plugins/claude" \
      --permission-mode bypassPermissions \
      --setting-sources "$SETTING_SOURCES" \
      --output-format stream-json --include-hook-events --verbose \
      "$A2_PROMPT" \
    >> "$A2_STREAM" 2> "$SCRATCH/a2-attempt${attempt}-stderr.txt" &
  A2_PID=$!
  track_pid "$A2_PID"
  echo "attempt $attempt pid: $A2_PID"

  sent=0
  deadline=$((SECONDS + 60))
  while [ $SECONDS -lt $deadline ]; do
    if [ -s "$A2_STREAM" ]; then
      count=$(jq -s '[.[] | select(.type=="system" and .subtype=="hook_response" and .hook_event=="PostToolUse")] | length' "$A2_STREAM" 2>/dev/null || echo 0)
      if [ "$count" -ge 2 ]; then
        set -x
        AGENT_MSG_NAME=qsl-lead agent-msg send tl-lead "stop-block check AGE-2011 (${marker})"
        set +x
        sent=1
        break
      fi
    fi
    sleep 0.1
  done
  [ "$sent" -eq 1 ] || echo "WARNING: never saw 2 PostToolUse responses within 60s on attempt $attempt"

  wait "$A2_PID" || true

  if [ "$sent" -eq 1 ] && python3 "$ANALYZE" "$A2_STREAM" stop_block_first "$marker"; then
    echo "attempt $attempt: WON the race"
    STOP_WON=1
    break
  else
    echo "attempt $attempt: LOST the race"
  fi
done

if [ "$STOP_WON" -eq 1 ]; then
  pass "stop-blocks" "Stop hook returned decision:block while a message was pending"
else
  fail "stop-blocks" "all 3 attempts lost the race or failed to send in time"
fi
echo

# ===========================================================================
# PHASE B: two live leads. qsl-lead is a second headless Claude session, not
# the CLI. Each side sends via its own agent-msg (run by the model, via the
# Bash tool) and reports what it received; message exchange is proven by
# both sessions' own hook streams and their own model output.
# ===========================================================================
echo "=== PHASE B: two live leads exchange messages in both directions ==="

TL_PROMPT='You are tl-lead, being tested for a message bus plugin. Do exactly these steps in order and nothing else.
1. Use the Bash tool to run: sleep 2
2. Use the Bash tool to run: agent-msg send qsl-lead "hello from tl-lead exchange test"
3. Use the Bash tool to run: sleep 8
4. Repeat verbatim any line starting with "[ix-relay]" that you have received.
5. Write one short sentence saying you are done, and stop.'

QSL_PROMPT='You are qsl-lead, being tested for a message bus plugin. Do exactly these steps in order and nothing else.
1. Use the Bash tool to run: sleep 1
2. Use the Bash tool to run: sleep 4
3. Repeat verbatim any line starting with "[ix-relay]" that you have received.
4. Use the Bash tool to run: agent-msg send tl-lead "hello back from qsl-lead exchange test"
5. Write one short sentence saying you are done, and stop.'

TL_STREAM="$SCRATCH/exchange-tl-stream.jsonl"
QSL_STREAM="$SCRATCH/exchange-qsl-stream.jsonl"
: > "$TL_STREAM"
: > "$QSL_STREAM"

AGENT_MSG_NAME=tl-lead IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
  claude -p --model haiku \
    --plugin-dir "$REPO_ROOT/plugins/claude" \
    --permission-mode bypassPermissions \
    --setting-sources "$SETTING_SOURCES" \
    --output-format stream-json --include-hook-events --verbose \
    "$TL_PROMPT" \
  >> "$TL_STREAM" 2> "$SCRATCH/exchange-tl-stderr.txt" &
TL_PID=$!
track_pid "$TL_PID"

AGENT_MSG_NAME=qsl-lead IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
  claude -p --model haiku \
    --plugin-dir "$REPO_ROOT/plugins/claude" \
    --permission-mode bypassPermissions \
    --setting-sources "$SETTING_SOURCES" \
    --output-format stream-json --include-hook-events --verbose \
    "$QSL_PROMPT" \
  >> "$QSL_STREAM" 2> "$SCRATCH/exchange-qsl-stderr.txt" &
QSL_PID=$!
track_pid "$QSL_PID"

echo "tl-lead pid: $TL_PID, qsl-lead pid: $QSL_PID, running concurrently"
wait "$TL_PID" || true
wait "$QSL_PID" || true
echo "-- both exchange sessions finished --"
echo

echo "-- exchange checks --"
if python3 "$ANALYZE" "$QSL_STREAM" hook_contains PostToolUse "hello from tl-lead exchange test" \
   && python3 "$ANALYZE" "$QSL_STREAM" assistant_contains "hello from tl-lead exchange test"; then
  pass "exchange-to-qsl" "qsl-lead's own live session received and reported tl-lead's message"
else
  fail "exchange-to-qsl" "see analyze.py output above"
fi

if python3 "$ANALYZE" "$TL_STREAM" hook_contains PostToolUse "hello back from qsl-lead exchange test" \
   && python3 "$ANALYZE" "$TL_STREAM" assistant_contains "hello back from qsl-lead exchange test"; then
  pass "exchange-to-tl" "tl-lead's own live session received and reported qsl-lead's reply"
else
  fail "exchange-to-tl" "see analyze.py output above"
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
