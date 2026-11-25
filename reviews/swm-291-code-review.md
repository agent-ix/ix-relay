---
id: SR-6123
title: "Code review — ix-relay SWM-291 Python scripts ported to relay-tools"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; crates/relay-tools/**, Cargo.lock, Makefile, CHANGELOG.md, CLAUDE.md, README.md, docs/how-it-works.md, plugins/codex/README.md, scripts/*.py (deleted), .github/workflows/release.yml (context)"
review_set: subset
---

# SR-6123: Code review — ix-relay SWM-291 Python scripts ported to relay-tools

## Summary

Ticket: SWM-291. PR agent-ix/ix-relay#65 at `2d773e9d8d9737f7c4ec475308b183674d09a6df`, reviewed as `git diff origin/main...HEAD` (26 files, +1000/-518). The rust-review lane is folded into this file. Each port was compared with the deleted Python read from origin/main, and the old and new tools were run side by side.

Evidence (macOS, CARGO_BUILD_JOBS=4, target inside the worktree, native `redis-server --save '' --appendonly no` on a random port via `IX_RELAY_TEST_URL`, no Docker):

- `cargo test -p relay-tools`: 11 unit tests, 2 archive tests (Redis-backed, ran) and 3 launcher tests pass.
- `cargo fmt --all -- --check` clean; `cargo clippy -p relay-tools --all-targets -- -D warnings` clean; `cargo deny check licenses bans` ok; `cargo doc -p relay-tools` with `-D warnings` clean; `make check-matrix` prints the same recount as the Python (373 rows).
- check-test-matrix differential run, deleted Python against the Rust binary, on 29 mutated copies of `spec/tests.md`. The coder's five mutations agree (same stdout, exit 1). The extra cases are listed under FND-002.
- archive-relay-namespace: percent-encoded ACL user and password, `/3` database selection, `--prefix=` inline form, the config-file fallback, refusal of `ixr:v3:`, and the exclusive 0600 output all behave like the Python.
- codex-herdr-launcher and capture-codex-session-start were run against the Python from origin/main with the same arguments and payloads.

## Verdict

**FAIL**: FND-001 is a duplication finding, and the code-review rule is that duplication is high severity.

What is correct: the session-start capture keeps the allowlist, the subagent skip, `O_NOFOLLOW`, the owner check and the 0600 mode. The launcher's option-value skipping matches the Python for UTF-8 arguments. The archive keeps exclusive creation, binary-safe keys and values, the TTL, the source left unchanged and the fixed failure line. Deleting `scripts/latency-bench.py` is justified: no file on origin/main refers to it apart from itself, and its docstring calls it a one-off harness for the AGE-2028 figures. `git ls-files` lists no `.py` file. The PR adds no logic-bearing shell: `make check-matrix` is now `cargo run -p relay-tools --bin check-test-matrix`. `plugins/codex/hooks.json` still runs only `agent-msg hook codex`. The capture hook in the README is an absolute path to a binary that the documented `cargo build --release -p relay-tools` produces. The installed launchers on draco, luna and titan are copies, not symlinks into `scripts/`, so deleting the scripts breaks no live `codex`. The release workflow builds with `-p agent-msg -p relay-watcher`, so the release tarballs do not get TLS (see FND-003).

## Findings

| ID      | Severity | Summary | Refs |
| ------- | -------- | ------- | ---- |
| FND-001 | high | `archive-relay-namespace` re-implements relay-core's relay URL and config resolution: non-empty `IX_RELAY_URL`, then `url` from `--config`, defaulting to `$HOME/.ix/relay/config.toml`. It does not use `relay_core::config` (`config_path`, `load_core`, `env_string`). These are two implementations of one rule, and they already differ: the bin ignores `IX_RELAY_CONFIG`, so an operator who redirects the config archives from a different Redis than `agent-msg` and `relay-watcher` use. The bin also skips relay-core's validation of the file, and with `HOME` unset it exits with the usage error even when `IX_RELAY_URL` is set. | crates/relay-tools/src/bin/archive-relay-namespace.rs:302-320; crates/relay-core/src/config.rs:275, 339, 504 |
| FND-002 | medium | check-test-matrix accepts files the Python rejected, and the PR body claims the opposite. `str::lines` splits only on `\n`, while Python's `splitlines` also splits on `\f`, `\x1c`, NEL, U+2028 and a lone `\r`. A `\| TC-` row placed after any of those characters in a traceability table passes the Rust gate with exit 0 (5 cases reproduced). Two numeric cases also exit 0 where Python exited 1: a superscript `²` total, and a total too large for `usize` (`99999999999999999999999`) on a category with no rows. The parse fails, so the row is skipped. | crates/relay-tools/src/matrix.rs:628, 636, 663-669 |
| FND-003 | medium | The redis `tls-native-tls` feature costs more than `rediss://` support is worth. Feature unification enables it for every workspace build: `make build` (`cargo build --release`) links native-tls/OpenSSL into `agent-msg` and `relay-watcher`. On Linux, `make ci`, `make check-matrix` (which now runs `cargo run -p relay-tools`) and the documented launcher and capture-hook build all compile `openssl-sys` and need libssl-dev and pkg-config. Workspace builds now contradict FR-045 ("No TLS library is in the tree") and docs/observability.md ("No TLS library is built in"). The only user is a one-off 0.3.0 namespace-archive tool, and no relay URL in the repo or deploy uses `rediss://`, because relay-redis is reached over WireGuard. Release tarballs (`-p agent-msg -p relay-watcher`) and `cargo install --git … agent-msg` are not affected. Either refuse `rediss://` with a clear message, or put TLS behind a non-default `tls` feature of relay-tools. | crates/relay-tools/Cargo.toml:18; Makefile:63, 102; spec/functional/FR-045-watcher-pushes-metrics-over-otlp.md:40; docs/observability.md:17 |
| FND-004 | low | The archive has no connect or read timeout. The Python set a 10 s socket timeout. Against a listener that accepts and never replies, the Rust binary was still blocked after 25 s. relay-core's `Store::connect(url, timeout)` and redis `get_connection_with_timeout` / `set_read_timeout` both exist. | crates/relay-tools/src/archive.rs:207 |
| FND-005 | low | The launcher drops non-UTF-8 arguments while scanning (`filter_map(to_str)`), which shifts the option-value skip. Reproduced with `HERDR_ENV=1`: `codex --model $'\xff' exec prompt` gets `--no-daemon` from Rust but passes through unchanged under the Python. Using `to_string_lossy` for the scan keeps the positions. | crates/relay-tools/src/launcher.rs:492-497 |
| FND-006 | low | The CHANGELOG says "with the same flags and output", but capture-codex-session-start writes its keys in a different order. `json!` builds a sorted map, so the line starts `{"context":{"CODEX_THREAD_ID":…` where the Python wrote `hook_event_name, session_id, transcript_path, context`. The archive's separators differ too. Both are equivalent JSON, so the claim should be "equivalent JSON". | crates/relay-tools/src/session_start.rs:858; CHANGELOG.md:5 |
| FND-007 | low | The released 0.3.0 CHANGELOG entry was rewritten to name the `archive-relay-namespace` binary, which did not exist in 0.3.0. Release notes should describe the release as shipped; the Unreleased entry already records the rename. | CHANGELOG.md:56 |
