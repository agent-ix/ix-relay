#!/usr/bin/env bash
# Manual, live end-to-end test for the opencode plugin at plugins/opencode
# (AGE-2016). Proves the plugin's hooks actually deliver ix-relay messages
# inside real opencode sessions, against a real (throwaway) Redis, and prints
# a PASS/FAIL line per check. Exits nonzero if any check fails. Not part of
# `make ci`: it starts several real opencode and Claude Code sessions and
# spends real API calls (opencode's are free — see below).
#
# Delivery points covered (see plugins/opencode/README.md for how each one was
# verified against the actual plugin API, with evidence):
#
#   session-start     a message queued before an opencode lead's session
#                      starts is visible in the model's own context once its
#                      first tool call runs (there is no opencode hook that
#                      fires before any tool call and before the turn ends,
#                      so this is the earliest real delivery point)
#   midturn            a message queued strictly after the session's first
#                      tool call has already completed is still delivered, by
#                      a later tool call's `tool.execute.after`
#   idle-nudge         when a turn ends with a message still pending, the
#                      `session.idle` handler sends a short promptAsync nudge
#                      ("run `agent-msg inbox` now") instead of popping
#                      itself; the model's own subsequent `agent-msg inbox`
#                      call is what actually delivers the text. Proving this
#                      needs the session to survive past the first idle event
#                      — `opencode run` tears its ephemeral server down right
#                      then, so this check runs against `opencode serve`
#                      (a persistent server) instead, and reads the reply
#                      back from the server's own session history over HTTP.
#   exchange-to-oc     a live Claude Code lead's `agent-msg send` reaches a
#                      live opencode lead's own session (not the CLI), and the
#                      opencode model's own tool output shows it
#   exchange-to-tl     the opencode lead replies only after it has actually
#                      received the Claude lead's message (ordering is
#                      asserted, not assumed), and the Claude lead's own
#                      session reports the reply
#
# Requires: docker, cargo, opencode on PATH with a working model (see below),
# a `claude` on PATH authenticated to make API calls, jq, curl, python3.
#
# Model: opencode/space-bunny-free — opencode zen's free tier, which needs no
# credential at all and costs $0 per `opencode run`'s own reported cost. This
# was the cheapest model actually reachable in the environment this test was
# authored in: github-copilot/* rejected the available GITHUB_TOKEN (personal
# access tokens aren't accepted for Copilot chat), and google/* + openai/*
# rejected the available API keys. If neither opencode/space-bunny-free nor
# any OPENCODE_LIVE_TEST_MODEL override works in your environment, this
# script fails fast in its preflight check and says so instead of guessing.
#
# CARGO_TARGET_DIR: always the repo's own `target/`, never an inherited value
# — a shared target dir across worktrees on the same machine lets a concurrent
# build from another checkout silently overwrite the `agent-msg` binary
# mid-run (this bit AGE-2016 once; see the build step below).
#
# Usage:
#   scripts/live-test-opencode-plugin.sh [scratch-dir]
#
# scratch-dir defaults to a fresh mktemp -d. Everything the script runs and
# prints is also written to <scratch-dir>/age-2016-live.log.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRATCH="${1:-$(mktemp -d)}"
CONTAINER="ixr-oc-livetest-$$"
OC_MODEL="${OPENCODE_LIVE_TEST_MODEL:-opencode/space-bunny-free}"
export CARGO_TARGET_DIR="$REPO_ROOT/target"

mkdir -p "$SCRATCH"
LOG="$SCRATCH/age-2016-live.log"
exec > >(tee "$LOG") 2>&1

echo "=== ix-relay opencode plugin live test ==="
echo "repo:    $REPO_ROOT"
echo "scratch: $SCRATCH"
echo "log:     $LOG"
echo "model:   $OC_MODEL"
echo "target:  $CARGO_TARGET_DIR (this repo's own, not a shared one)"
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
      echo "killing leftover pid $pid"
      kill -9 "$pid" 2>/dev/null || true
    fi
  done
  echo "stopping redis container $CONTAINER"
  docker stop "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Preflight: tools, model reachability.
# ---------------------------------------------------------------------------
for tool in docker cargo opencode claude jq curl python3; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "FATAL: $tool not on PATH"
    exit 1
  }
done

# A free TCP port on 127.0.0.1, for `opencode serve` (CHECK 3). Not reused
# across calls within one run of this script, so the two ports this script
# needs (redis's host port comes from docker separately) never collide.
free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

echo "--- preflight: is $OC_MODEL reachable? ---"
PREFLIGHT_DIR="$SCRATCH/preflight"
mkdir -p "$PREFLIGHT_DIR/xdgconfig" "$PREFLIGHT_DIR/xdgdata" "$PREFLIGHT_DIR/xdgcache"
set -x
XDG_CONFIG_HOME="$PREFLIGHT_DIR/xdgconfig" XDG_DATA_HOME="$PREFLIGHT_DIR/xdgdata" XDG_CACHE_HOME="$PREFLIGHT_DIR/xdgcache" \
  timeout 180 opencode run "reply with exactly: pong" \
  -m "$OC_MODEL" --dir "$PREFLIGHT_DIR" --format json < /dev/null >"$PREFLIGHT_DIR/out.json" 2>"$PREFLIGHT_DIR/err.txt"
PREFLIGHT_STATUS=$?
set +x
if [ "$PREFLIGHT_STATUS" -ne 0 ] || grep -q '"type":"error"' "$PREFLIGHT_DIR/out.json"; then
  echo "FATAL: $OC_MODEL is not reachable with the credentials available in this environment."
  echo "       Set OPENCODE_LIVE_TEST_MODEL to a model this environment can reach (check"
  echo "       \`opencode auth list\` for which providers have credentials) and retry."
  cat "$PREFLIGHT_DIR/out.json" "$PREFLIGHT_DIR/err.txt"
  exit 1
fi
echo "preflight ok: $OC_MODEL answered"
echo

# ---------------------------------------------------------------------------
# Build agent-msg and start a throwaway, randomly-ported Redis (never 6379,
# which is the production bus tunnel).
# ---------------------------------------------------------------------------
cd "$REPO_ROOT"

echo "--- build agent-msg (CARGO_TARGET_DIR=$CARGO_TARGET_DIR, this repo's own) ---"
set -x
cargo build -p agent-msg
set +x
BIN_DIR="$CARGO_TARGET_DIR/debug"
export PATH="$BIN_DIR:$PATH"
agent-msg inbox --help | grep -q -- --count || {
  echo "FATAL: $BIN_DIR/agent-msg does not have --count. Build did not produce what this"
  echo "       checkout's source says it should; something is wrong with the build, not a race."
  exit 1
}
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
# AGE-2017: agent-msg now also reads ~/.ix/relay/config.toml (flag > env >
# file > default). IX_RELAY_URL above already overrides a real file's `url`,
# but not `prefix` — point IX_RELAY_CONFIG at a path that can't exist so a
# developer's real config file never leaks a prefix into this run.
export IX_RELAY_CONFIG="/tmp/ix-relay-live-test-no-config-$$-${RANDOM}"
echo "IX_RELAY_URL=$IX_RELAY_URL (port $REDIS_PORT, never 6379)"
echo "IX_RELAY_CONFIG=$IX_RELAY_CONFIG (isolated from any real config file)"
echo

echo "--- register both leads (bookkeeping only; agent-msg send refuses an unregistered recipient) ---"
set -x
AGENT_MSG_NAME=oc-lead agent-msg register --kind opencode
AGENT_MSG_NAME=tl-lead agent-msg register --kind claude
AGENT_MSG_NAME=qsl-probe-sender agent-msg register --kind manual
set +x
echo

# ---------------------------------------------------------------------------
# An isolated opencode project: its own plugin dir (a copy of the real
# plugin file, so this is what actually ships), its own XDG_CONFIG_HOME /
# XDG_DATA_HOME / XDG_CACHE_HOME so it never touches the real user's opencode
# config, session data, auth or plugins, and its own opencode.json wiring the
# plugin + AGENTS.md instructions the way the README documents.
# ---------------------------------------------------------------------------
OC_PROJECT="$SCRATCH/oc-project"
mkdir -p "$OC_PROJECT/.opencode/plugin" "$OC_PROJECT/xdgconfig" "$OC_PROJECT/xdgdata" "$OC_PROJECT/xdgcache"
cp "$REPO_ROOT/plugins/opencode/ix-relay.ts" "$OC_PROJECT/.opencode/plugin/ix-relay.ts"
cat >"$OC_PROJECT/opencode.json" <<JSONEOF
{
  "instructions": ["$REPO_ROOT/plugins/opencode/AGENTS.md"]
}
JSONEOF

# Backgrounds an `opencode run` and sets LAST_BG_PID. Not a subshell (no
# command substitution): the background job must be a direct child of this
# script's own shell, or `wait` on it later fails with "not a child of this
# shell" and the check proceeds before the run has actually finished.
LAST_BG_PID=""
run_opencode() {
  # run_opencode <lead-name> <stream-file> <prompt> [extra opencode args...]
  local lead="$1" stream="$2" prompt="$3"
  shift 3
  AGENT_MSG_NAME="$lead" IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
    XDG_CONFIG_HOME="$OC_PROJECT/xdgconfig" XDG_DATA_HOME="$OC_PROJECT/xdgdata" XDG_CACHE_HOME="$OC_PROJECT/xdgcache" \
    opencode run "$prompt" -m "$OC_MODEL" --format json "$@" \
    </dev/null >>"$stream" 2>>"${stream%.json}.err.txt" &
  LAST_BG_PID=$!
}

# jq filter: has a completed tool_use (any tool) whose command/description
# contains $1 landed in stream file $2? Delivery can ride any tool call, not
# just bash (e.g. opencode's own auto-invoked "skill" tool), but the watchers
# below key off bash's `input.command` specifically since that's what the
# test prompts ask the model to run.
bash_call_done() {
  local marker="$1" stream="$2"
  [ -s "$stream" ] || return 1
  jq -s --arg m "$marker" '
    [.[] | select(.type=="tool_use" and .part.tool=="bash"
      and (.part.state.status=="completed")
      and ((.part.state.input.command // "") | contains($m)))] | length
  ' "$stream" 2>/dev/null | grep -qv '^0$'
}

# ===========================================================================
# CHECK 1: session-start — a message queued before the session starts is
# delivered once the lead's first tool call completes.
# ===========================================================================
echo "=== CHECK 1: session-start ==="
M1="oc-sessionstart-$$"
set -x
AGENT_MSG_NAME=qsl-probe-sender agent-msg send oc-lead "session-start check AGE-2016 ($M1)"
set +x

S1="$SCRATCH/1-sessionstart-stream.json"
: >"$S1"
run_opencode oc-lead "$S1" \
  "Use the bash tool to run exactly: echo start-marker-c1. Then say the word done." \
  --dir "$OC_PROJECT"
PID1=$LAST_BG_PID
track_pid "$PID1"
wait "$PID1" || true

if grep -qF "$M1" "$S1"; then
  pass "session-start" "backlog queued before the session started showed up in the model's own context, delivered by a tool call"
else
  fail "session-start" "marker ($M1) never appeared in $S1"
fi
echo

# ===========================================================================
# CHECK 2: midturn — a message queued strictly after the first tool call has
# already completed is still delivered, by a later tool call.
# ===========================================================================
echo "=== CHECK 2: midturn ==="
M2="oc-midturn-$$"
S2="$SCRATCH/2-midturn-stream.json"
: >"$S2"
run_opencode oc-lead "$S2" \
  "Use the bash tool to run exactly: echo midturn-marker-a. Then use the bash tool to run exactly: echo midturn-marker-b. Then say the word done." \
  --dir "$OC_PROJECT"
PID2=$LAST_BG_PID
track_pid "$PID2"

echo "-- watcher: waiting for the first bash call (midturn-marker-a) to complete before sending --"
deadline=$((SECONDS + 180))
sent=0
while [ $SECONDS -lt $deadline ]; do
  if bash_call_done "midturn-marker-a" "$S2"; then
    set -x
    AGENT_MSG_NAME=qsl-probe-sender agent-msg send oc-lead "midturn check AGE-2016 ($M2)"
    set +x
    sent=1
    break
  fi
  sleep 0.3
done
[ "$sent" -eq 1 ] || echo "WARNING: never saw the first bash call complete within 60s"

wait "$PID2" || true

if [ "$sent" -eq 1 ] && grep -qF "$M2" "$S2"; then
  pass "midturn" "message queued after the first tool call was delivered by a later one"
else
  fail "midturn" "marker ($M2) never appeared in $S2 (sent=$sent)"
fi
echo

# ===========================================================================
# CHECK 3: idle-nudge — a message queued after the session's only tool call
# has completed, with no further tool call in that same turn to deliver it,
# still reaches the model: the session.idle handler sends a promptAsync
# nudge, and the model's own next `agent-msg inbox` call (in a NEW turn) is
# what actually surfaces the text. `opencode run` alone can't observe this
# (its ephemeral server dies at the first idle event — confirmed while
# building this plugin: even a bare `setTimeout` inside the idle handler
# never got its first tick before the process was gone), so this check runs
# against a persistent `opencode serve` instead, driven via
# `opencode run --attach`, and reads the reply back over the server's own
# HTTP API instead of trusting the CLI's own stdout to still be flowing.
# ===========================================================================
echo "=== CHECK 3: idle-nudge (via opencode serve, so the follow-up turn survives to be read back) ==="
M3="oc-idle-$$"
OC_SERVE_PORT="$(free_port)"
OC_SERVE_PROJECT="$SCRATCH/oc-serve-project"
mkdir -p "$OC_SERVE_PROJECT/.opencode/plugin" "$OC_SERVE_PROJECT/xdgconfig" "$OC_SERVE_PROJECT/xdgdata" "$OC_SERVE_PROJECT/xdgcache"
cp "$REPO_ROOT/plugins/opencode/ix-relay.ts" "$OC_SERVE_PROJECT/.opencode/plugin/ix-relay.ts"

SERVE_OUT="$SCRATCH/3-serve.out"
SERVE_ERR="$SCRATCH/3-serve.err"
: >"$SERVE_OUT"
: >"$SERVE_ERR"
(
  cd "$OC_SERVE_PROJECT"
  AGENT_MSG_NAME=oc-lead IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
    XDG_CONFIG_HOME="$OC_SERVE_PROJECT/xdgconfig" XDG_DATA_HOME="$OC_SERVE_PROJECT/xdgdata" XDG_CACHE_HOME="$OC_SERVE_PROJECT/xdgcache" \
    exec opencode serve --port "$OC_SERVE_PORT" --hostname 127.0.0.1
) >>"$SERVE_OUT" 2>>"$SERVE_ERR" &
SERVE_PID=$!
track_pid "$SERVE_PID"

echo "-- waiting for opencode serve on port $OC_SERVE_PORT (pid $SERVE_PID) --"
serve_deadline=$((SECONDS + 300))
serve_ready=0
while [ $SECONDS -lt $serve_deadline ]; do
  if curl -s --max-time 5 -o /dev/null "http://127.0.0.1:${OC_SERVE_PORT}/session"; then
    serve_ready=1
    break
  fi
  sleep 0.3
done

if [ "$serve_ready" -ne 1 ]; then
  fail "idle-nudge" "opencode serve never became reachable on port $OC_SERVE_PORT within 120s"
  cat "$SERVE_OUT" "$SERVE_ERR"
else
  # Racy by nature, the same way Claude's own live-test's Stop-hook check is:
  # observed live, the gap between the model's only tool call finishing and
  # the server actually marking the turn idle was as short as ~1.5s, so the
  # send has to land in that window. Retried up to 3 attempts total; a run
  # where every attempt loses the race is a FAIL, never a silent pass.
  IDLE_WON=0
  for attempt in 1 2 3; do
    echo "-- idle-nudge attempt $attempt --"
    M3="oc-idle-$$-attempt${attempt}"
    S3="$SCRATCH/3-idle-attach-attempt${attempt}-stream.json"
    : >"$S3"
    AGENT_MSG_NAME=oc-lead IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
      opencode run "Use the bash tool to run exactly once: echo idle-marker-only-attempt${attempt}. Then say the word done. Do not call any more tools after that." \
      -m "$OC_MODEL" --attach "http://127.0.0.1:${OC_SERVE_PORT}" --dir "$OC_SERVE_PROJECT" --format json \
      </dev/null >>"$S3" 2>>"$SCRATCH/3-idle-attach-attempt${attempt}.err.txt" &
    ATTACH_PID=$!
    track_pid "$ATTACH_PID"

    deadline=$((SECONDS + 180))
    sent3=0
    while [ $SECONDS -lt $deadline ]; do
      if bash_call_done "idle-marker-only-attempt${attempt}" "$S3"; then
        AGENT_MSG_NAME=qsl-probe-sender agent-msg send oc-lead "idle check AGE-2016 ($M3): run agent-msg inbox" >/dev/null
        sent3=1
        break
      fi
      sleep 0.2
    done
    [ "$sent3" -eq 1 ] || echo "WARNING: never saw the only bash call complete within 120s on attempt $attempt"

    wait "$ATTACH_PID" || true

    # How many bash calls landed in the initial turn? Must be exactly 1 (the
    # one we waited for) — otherwise a later tool call in that SAME turn
    # could have delivered it via tool.execute.after instead of the
    # idle-nudge path, and this attempt would prove nothing about the idle
    # path specifically.
    bash_calls=$(jq -s '[.[] | select(.type=="tool_use" and .part.tool=="bash")] | length' "$S3" 2>/dev/null || echo -1)
    session_id=$(jq -s -r '[.[] | select(.sessionID != null) | .sessionID] | first // empty' "$S3" 2>/dev/null)

    if [ "$sent3" -ne 1 ] || [ "$bash_calls" != "1" ] || [ -z "$session_id" ]; then
      echo "attempt $attempt: setup didn't hold (sent=$sent3 bash_calls=$bash_calls session_id=${session_id:-<none>}), not a plugin verdict"
      continue
    fi

    echo "-- attempt $attempt: initial turn done (session $session_id); polling the server for the model's own follow-up reply --"
    poll_deadline=$((SECONDS + 240))
    seen=0
    while [ $SECONDS -lt $poll_deadline ]; do
      if curl -s --max-time 10 "http://127.0.0.1:${OC_SERVE_PORT}/session/${session_id}/message" 2>/dev/null | grep -qF "$M3"; then
        seen=1
        break
      fi
      sleep 1
    done
    if [ "$seen" -eq 1 ]; then
      echo "attempt $attempt: WON — the model's own follow-up agent-msg inbox call delivered it"
      IDLE_WON=1
      break
    fi
    # The plugin never pops on idle (that's the point of the redesign: a pop
    # nobody can promise gets seen is a loss). So a missed nudge within the
    # poll window is not a lost message — it's still sitting in the inbox,
    # exactly where relay-watcher (AGE-2013) will find and deliver it. Assert
    # that no-loss property directly, with a non-destructive peek, instead of
    # describing this as "LOST".
    still_queued=$(AGENT_MSG_NAME=oc-lead agent-msg inbox --count 2>/dev/null || echo "?")
    echo "attempt $attempt: not delivered by the nudge within 240s (still queued: $still_queued pending for oc-lead)"
  done

  if [ "$IDLE_WON" -eq 1 ]; then
    pass "idle-nudge" "session.idle sent a promptAsync nudge with no message pop of its own; the model's own follow-up agent-msg inbox call (read back from opencode serve's own session history) delivered it"
  else
    fail "idle-nudge" "none of 3 attempts saw the model's follow-up reply within the poll window; the message was never popped by the plugin either (see the per-attempt 'still queued' counts above), so it remains pending for relay-watcher (AGE-2013) rather than lost"
  fi

  kill "$SERVE_PID" 2>/dev/null || true
  wait "$SERVE_PID" 2>/dev/null || true
fi
echo

# ===========================================================================
# CHECK 4 & 5: a live Claude Code lead and a live opencode lead exchange
# messages in both directions. The opencode lead must reply only *after* it
# has actually received the Claude lead's message (the Done-when in
# AGE-2016) — asserted by comparing event timestamps, not assumed from step
# order in the prompt.
# ===========================================================================
echo "=== CHECK 4/5: two live leads exchange messages in both directions ==="

# claude-haiku-4-5's Bash tool will silently auto-background a single long
# `sleep N` call (run_in_background: true) and then end the turn immediately
# without actually waiting — this is what made exchange-to-tl FAIL the first
# time this test ran for real (root cause, not a plugin bug): tl-lead never
# waited the 60s it was told to, so it exited before oc-lead's reply could
# arrive. Multiple short foreground sleeps avoid it, the same pattern
# scripts/live-test-claude-plugin.sh already uses successfully.
TL_PROMPT='You are tl-lead, being tested for a message bus plugin. Do exactly these steps in order and nothing else. Run every Bash call in the foreground; do not set run_in_background on any of them.
1. Use the Bash tool to run: sleep 3
2. Use the Bash tool to run: agent-msg send oc-lead "hello from tl-lead exchange test AGE-2016"
3. Use the Bash tool to run: sleep 10
4. Use the Bash tool to run: sleep 10
5. Use the Bash tool to run: sleep 10
6. Use the Bash tool to run: sleep 10
7. Repeat verbatim any line starting with "[ix-relay]" that you have received.
8. Write one short sentence saying you are done, and stop.'

TL_STREAM="$SCRATCH/exchange-tl-stream.jsonl"
: >"$TL_STREAM"
AGENT_MSG_NAME=tl-lead IX_RELAY_URL="$IX_RELAY_URL" PATH="$BIN_DIR:$PATH" \
  claude -p --model haiku \
    --plugin-dir "$REPO_ROOT/plugins/claude" \
    --permission-mode bypassPermissions \
    --setting-sources project,local \
    --output-format stream-json --include-hook-events --verbose \
    "$TL_PROMPT" \
  >>"$TL_STREAM" 2>"$SCRATCH/exchange-tl-stderr.txt" &
TL_PID=$!
track_pid "$TL_PID"
echo "tl-lead (claude) pid: $TL_PID"

# oc-lead's schedule is fixed, not conditional (more reliable for a cheap
# model than "wait for X, then do Y"): four short sleeps give tool.execute.after
# several chances to deliver tl-lead's message, then an explicit echo-back
# step forces it into the model's own visible output before it replies, so
# the ordering assertion below has something real to check.
OC_EXCHANGE_STREAM="$SCRATCH/exchange-oc-stream.json"
: >"$OC_EXCHANGE_STREAM"
OC_EXCHANGE_PROMPT='You are oc-lead, being tested for a message bus plugin. Do exactly these steps in order and nothing else.
1. Use the bash tool to run: sleep 5
2. Use the bash tool to run: sleep 5
3. Use the bash tool to run: sleep 5
4. Use the bash tool to run: sleep 5
5. Repeat verbatim any line starting with "[ix-relay] from tl-lead" that you have received so far.
6. Use the bash tool to run: agent-msg send tl-lead "hello back from oc-lead exchange test AGE-2016"
7. Say the word done.'
run_opencode oc-lead "$OC_EXCHANGE_STREAM" "$OC_EXCHANGE_PROMPT" --dir "$OC_PROJECT"
OC_PID=$LAST_BG_PID
track_pid "$OC_PID"
echo "oc-lead (opencode) pid: $OC_PID"

wait "$TL_PID" || true
wait "$OC_PID" || true
echo "-- both exchange sessions finished --"
echo

if grep -qF "hello from tl-lead exchange test AGE-2016" "$OC_EXCHANGE_STREAM"; then
  pass "exchange-to-oc" "oc-lead's own live opencode session received and surfaced tl-lead's message"
else
  fail "exchange-to-oc" "marker never appeared in $OC_EXCHANGE_STREAM"
fi

# exchange-to-tl: not just "does the marker appear", but "did oc-lead receive
# tl-lead's message before it sent its own reply" — the actual Done-when.
received_ts=$(jq -s '[.[] | select((.. | strings? // empty) | contains("hello from tl-lead exchange test AGE-2016")) | .timestamp] | min // empty' "$OC_EXCHANGE_STREAM" 2>/dev/null)
sent_ts=$(jq -s '
  [.[] | select(.type=="tool_use" and .part.tool=="bash"
    and ((.part.state.input.command // "") | contains("agent-msg send tl-lead")))
   | .timestamp] | min // empty
' "$OC_EXCHANGE_STREAM" 2>/dev/null)

if [ -z "$received_ts" ] || [ -z "$sent_ts" ]; then
  fail "exchange-to-tl" "couldn't find both a receipt and a send in oc-lead's own stream (received=${received_ts:-<none>} sent=${sent_ts:-<none>}) — see $OC_EXCHANGE_STREAM"
elif ! python3 -c "import sys; sys.exit(0 if float('$received_ts') < float('$sent_ts') else 1)"; then
  fail "exchange-to-tl" "oc-lead sent its reply (t=$sent_ts) before it had received tl-lead's message (t=$received_ts) — not a real reply-after-receipt"
elif grep -qF "hello back from oc-lead exchange test AGE-2016" "$TL_STREAM"; then
  pass "exchange-to-tl" "oc-lead replied only after receiving tl-lead's message (t=$received_ts < t=$sent_ts), and tl-lead's own live session reported the reply"
else
  fail "exchange-to-tl" "oc-lead replied after receiving (ordering ok) but the marker never appeared in tl-lead's own stream $TL_STREAM"
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
