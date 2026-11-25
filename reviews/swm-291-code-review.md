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

## New findings (disposition pass 1)

Head `c1945bdb1530cf5892b3209bff4c4bae5118f32f`, fix commit `c1945bd` reviewed as `git diff 2d773e9..c1945bd`.

| ID      | Severity | Summary | Refs |
| ------- | -------- | ------- | ---- |
| FND-008 | medium | `archive-relay-namespace` gives up on an unresponsive server after about 20 s, but FR-073 says it fails within 10 seconds. `IO_TIMEOUT` is applied twice in sequence, once to the connection handshake (`get_connection_with_timeout`) and once to the first SCAN read (`set_read_timeout`), so a server that accepts and never replies costs both. Measured 20 s against a listener that accepts and stays silent. The test `a_server_that_never_replies_fails_within_the_timeout` uses 300 ms and asserts under 2 s, so it cannot catch the doubling. | crates/relay-tools/src/archive.rs:96-98; spec/functional/FR-073-operator-tools.md:30; crates/relay-tools/tests/archive.rs:80-98 |

## Dispositions

| FND | outcome | reason |
| --- | ------- | ------ |
| FND-001 | fixed | The bin now resolves the URL with `relay_core::config::load_core(CoreFlags::default())` and drops `--config`, so `IX_RELAY_CONFIG`, the file validation and the case with `HOME` unset all behave as in agent-msg. Verified by running the binary: `IX_RELAY_CONFIG` with no `HOME` archives; `--config` gives the usage error. |
| FND-002 | fixed | The recount now splits lines like Python's `splitlines` and compares totals as exact integers. The original 29 differential cases and 10 new ones (`\v`, U+2029, leading zeros, `½`, `abc`, `+121`, `1 21`, `1_21`, a form feed inside the Test Case Summary) were run against the origin/main Python: no case exits 0 where the Python exited 1. Rust is stricter on 3 (Arabic-Indic digits ×2, `½`). |
| FND-003 | fixed | `tls-native-tls` was removed and `rediss://` is refused with a message saying why. `cargo tree --workspace --target all -i native-tls` matches nothing, and native-tls, openssl-probe and schannel are gone from Cargo.lock. |
| FND-004 | fixed | Connect, read and write timeouts are now set (`IO_TIMEOUT` = 10 s); the binary failed after 20 s instead of hanging. The 2× bound is FND-008. |
| FND-005 | fixed | The scan uses `to_string_lossy`. `--model $'\xff' exec prompt` with `HERDR_ENV=1` now passes through unchanged, as the Python did. The new unit test covers it. |
| FND-006 | fixed | The CHANGELOG now says the archive and capture tools write equivalent JSON with a different key order and separators. |
| FND-007 | fixed | The 0.3.0 entry again names `scripts/archive-relay-namespace.py`. |
| FND-008 | fixed | Round 2, head `d2ec8901534af936d6c4241e6e38b223fa009a92`. `connect_within` waits on the handshake thread with `recv_timeout` until a single deadline, and the first SCAN read gets only the time left. Measured with the binary against a silent listener: it fails after 10.25 s, process start included (before the fix, 20 s). The abandoned thread cannot hold the process open, because returning from `main` exits the process and its other threads. It holds only a socket and the client, and if it finishes late it drops the connection on a failed `send`. The test (1 s deadline, asserts under 1.5 s) passed 30 of 30 runs at load 13–17 and 30 of 30 with 36 extra busy processes (load 53 on 18 cores). |
