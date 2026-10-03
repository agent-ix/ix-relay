#!/usr/bin/env python3
"""Optional diagnostic hook: record only allowlisted SessionStart metadata."""
import json
import os
from pathlib import Path
import stat
import sys

CONTEXT_KEYS = (
    "HERDR_ENV", "HERDR_SOCKET_PATH", "HERDR_PANE_ID", "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID", "CODEX_THREAD_ID",
)


def main():
    payload = json.loads(sys.stdin.read(1024 * 1024))
    if payload.get("hook_event_name") != "SessionStart" or payload.get("agent_id"):
        return
    record = {
        key: payload.get(key)
        for key in ("hook_event_name", "session_id", "transcript_path")
    }
    record["context"] = {key: os.environ.get(key) for key in CONTEXT_KEYS}
    codex_home = Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex")))
    output = codex_home / "ix-relay" / "session-start.jsonl"
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            return
        os.fchmod(fd, 0o600)
        os.write(fd, (json.dumps(record) + "\n").encode())
    finally:
        os.close(fd)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, TypeError, AttributeError, OSError):
        # Diagnostics must not interrupt the real integration hooks.
        pass
