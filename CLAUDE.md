# ix-relay

Lead-to-lead message bus and escalation inbox for the agent swarm (agent-msg CLI).

## Commands

```bash
make fmt            # format with rustfmt
make fmt-check      # verify formatting (CI gate)
make lint           # clippy with -D warnings
make test           # cargo test (Redis tests skip without IX_RELAY_TEST_URL)
make test-redis     # all tests against a throwaway redis:7 container (docker)
make fuzz           # property-based fuzz lanes (`fuzz_*` ignored tests) at FUZZ_CASES cases; see fuzz/README.md
make build          # release build
make clean          # cargo clean
make deny           # cargo deny check licenses
make audit-unsafe   # check that every unsafe block has a // SAFETY: comment
make check-matrix   # recount spec/tests.md against its summary tables
make ci             # fmt-check + lint + test-redis + deny + audit-unsafe + docs + check-matrix (python3, stdlib only, is a prerequisite)
```

## Safety scaffolding

Backported from `agent-ix/ecaz`:

- `clippy.toml` pins MSRV to `1.98.1` (ix-cli-kit's floor, AGE-2017; `rust-toolchain.toml` and `[workspace.package].rust-version` move with it) and caps cognitive complexity / arg count
- `deny.toml` allow-lists licenses and denies unknown registries/git sources
- `scripts/check_unsafe_comments.sh` runs in CI and locally via `make audit-unsafe`. Every `unsafe {` block must have a `// SAFETY:` comment within the 3 preceding lines, or be listed in `scripts/unsafe_comment_baseline.txt`. Update the baseline with `bash scripts/check_unsafe_comments.sh --update-baseline`.
- `rustfmt.toml` uses 100-char width and `StdExternalCrate` import grouping. CI fails on drift.
- `rust-toolchain.toml` pins to `1.98.1` + rustfmt + clippy.

## Layout

```
crates/relay-core/     # Redis store + message/registration/escalation types; tests/store.rs needs Redis
crates/agent-msg/      # CLI and hook handler (bin: agent-msg)
crates/relay-watcher/  # one process per machine, wakes idle leads via herdr (bin: relay-watcher)
crates/claude-wire/    # Claude Code's cross-session wire formats: typed, pure parsing, drift detection (lib, no I/O)
crates/herdr-cli/      # shared herdr process-running/JSON-envelope plumbing (lib, no bin)
deploy/                # systemd/launchd units for relay-watcher (not installed by this repo)
scripts/               # local tooling, incl. test-with-redis.sh
spec/                  # requirements artifacts
```
