---
id: SR-6108
title: "Code review — ix-relay SWM-267 Linux x86_64 release binaries"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay; .github/workflows/release.yml"
review_set: subset
---

## Summary

Ticket: SWM-267. PR agent-ix/ix-relay#57 at `c31484a90e508282cd2933d0c9daa722298f4b69`, reviewed as `git diff origin/main...HEAD`. The diff adds one job, `linux-binaries`, to `.github/workflows/release.yml`. It builds `agent-msg` and `relay-watcher` for `x86_64-unknown-linux-gnu`, packs each with LICENSE into a tarball, writes a `.sha256` per tarball and attaches them to the tag's GitHub release.

Evidence: `actionlint .github/workflows/release.yml` exits 0. glibc measured over ssh: titan 2.43, luna 2.39, terra WSL 2.35. `rustup toolchain install --help` (rustup 1.29.0) says it installs "by default the active toolchain", which is the rust-toolchain.toml pin (1.98.1).

## Verdict

**CHANGES REQUESTED** for FND-001, or a measured disproof of it. FND-002 is low.

Checked and clean:

- **Triggers.** The workflow runs on `v*.*.*` tags and `workflow_dispatch` only. It never runs on a PR or push to a branch. The new job has `if: github.event_name == 'push'`, so a manual dispatch skips it. Plugin tags such as `ix-relay--v0.3.5` do not match `v*.*.*`.
- **Toolchain.** `rustup toolchain install` with no argument installs the toolchain from rust-toolchain.toml (rustup 1.28 and later; ubuntu-latest ships a newer rustup).
- **Locked build.** `cargo build --release --locked -p agent-msg -p relay-watcher`. Both packages produce binaries with those names. All git dependencies (ix-cli-kit, ix-board, ix-trace-rs) are public, so CI needs no git auth.
- **Asset naming and checksum.** `<bin>-${GITHUB_REF_NAME}-${TARGET}.tar.gz` with LICENSE at the tarball root matches the ix-board darwin asset (`ix-board-v0.7.2-aarch64-apple-darwin.tar.gz`, entries `ix-board`, `LICENSE`). `sha256sum` run inside `dist` writes `<hash>  <basename>`, the same format as the darwin `.sha256`. ix-relay has no darwin release asset today, so the ix-board darwin asset is the reference.
- **Ordering.** `needs: release` runs the job after the `release` job has run `gh release create`, and `softprops/action-gh-release@v2` uploads to that existing release. `GITHUB_REF_NAME` reaches shell through env, not through `${{ }}` interpolation.
- **Action pins.** Major-version tags (`actions/checkout@v5`, `Swatinem/rust-cache@v2`, `softprops/action-gh-release@v2`) match the file's existing `@v5`/`@v4` style.

## Findings

| ID | Severity | Summary | Refs |
| --- | --- | --- | --- |
| FND-001 | medium | The gnu binaries are built on `ubuntu-latest` (24.04, glibc 2.39). terra's WSL, a rollout host in this ticket, has glibc 2.35. Bundled C code (libsqlite3-sys, ring) compiled against 2.39 headers can require symbols newer than 2.35 (for example `__isoc23_strtol@GLIBC_2.38`), and the binary then fails to start on terra. `ubuntu-latest` also moves the floor up without notice. Build on `ubuntu-22.04`, or check `objdump -T` of the artifact for symbols above GLIBC_2.35. | .github/workflows/release.yml:73 |
| FND-002 | low | `linux-binaries` inherits the workflow-level `packages: write`, which it does not need. It only uploads release assets. A job-level `permissions: contents: write` would make it least-privilege. | .github/workflows/release.yml:13-15,70-73 |

## Dispositions

| FND | Outcome | reason |
| --- | --- | --- |
| FND-001 | fixed | 2867fcc: `linux-binaries` now `runs-on: ubuntu-22.04` (glibc 2.35, terra WSL floor). actionlint exit 0. |
| FND-002 | fixed | 2867fcc: job-level `permissions: contents: write` overrides the workflow-level `packages: write`. |
