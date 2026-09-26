---
id: SR-006
title: "Code review (with Rust lane) of ix-relay PR #29, AGE-2056"
type: SpecReview
analysis: code-review
scope: "agent-ix/ix-relay@b10190314cfd711e6f7f470eb7251cc265439f91; crates/relay-core/src/naming.rs, crates/relay-core/tests/names.rs, crates/relay-ops/tests/collision.rs, fuzz/README.md, spec/tests.md"
review_set: subset
---
# SR-006: Code review (with Rust lane) of ix-relay PR #29

## Summary

Ticket: AGE-2056. Round-2 review of PR #29 at head `b101903`, scoped to
`git diff origin/main...HEAD` (5 files). The fix to `with_suffix` is correct:
when the cut and re-append rebuild the base (proved to happen only at 31 or
32 bytes with a base that ends in `-<suffix>`), the head's first letter
changes, and the suffix stays whole. The reviewer's own sweep (52,608 bases,
each with every n from 1 to 1000, 52.6M calls, plus a widen sweep of 121,035
calls with suffixes of 1 to 30 bytes) found no invalid, over-long,
equal-to-base, non-suffix-terminated, or `--`-introducing result, and no
repeated numbered variant. The findings are about test strength, one
overstated doc claim, and naming ergonomics. None blocks merge.

Files examined: `crates/relay-core/src/naming.rs` (with_suffix, free_variant,
slug_body, the unit proptest), `crates/relay-core/src/name.rs` (grammar,
`ALL`, `from_constructed`), `crates/relay-core/src/ids.rs` (MachineName, the
widen source), `crates/relay-ops/src/naming.rs` (resolve_collision, the
caller), `crates/relay-core/tests/names.rs`, `crates/relay-ops/tests/collision.rs`,
`fuzz/README.md`, `spec/tests.md`, `Makefile`, `scripts/test-with-redis.sh`.

## Findings

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-001 | low | `check_with_suffix` says it checks "ending in the whole cleaned suffix" but asserts no tail. It checks only valid, <=32, != base, no leading `-`, no trailing `--`. The seed corpus includes the round-1 F-1 counterexamples (`a-`+30b with a 30b suffix, `a-`+29b with 29b), and the corpus test still passes when run against round-1 `naming.rs` (b4d4197), the code that cut the suffix (mutation run). So the fuzz lanes and the corpus cannot catch a suffix-truncation regression. The exhaustive grid and the explicit regression test do catch it. | crates/relay-core/tests/names.rs:512-525, crates/relay-core/tests/names.rs:529-555 |
| FND-002 | low | The new doc comment says "Every result still ends in its own whole `-<suffix>`, so variants for different suffixes stay distinct". That is false when one suffix is a dash-suffix of another. A widen suffix (a machine name such as `host-9` or `0-9`) and the numbered n=9 can produce the same variant. Example: base `aaaaaaaaaaaaaaaaaaaaaaaaaaa-0-9` (31 bytes) with widen `0-9` gives `baaaaaaaaaaaaaaaaaaaaaaaaaa-0-9`, and n=9 gives the same name. The non-rebuild shape already existed on main: base 27 `a` + `-0-zz` with widen `0-9` and n=9 both give 27 `a` + `-0-9`. 3,713 of the reviewer's 121,035 widen-sweep inputs showed it. Impact: `free_variant` asks the same name twice, which costs one registry read and leaves 998 distinct numbered candidates after the widen. No taken name is returned, because `usable` re-checks. | crates/relay-core/src/naming.rs:209-211, crates/relay-core/src/naming.rs:233-247 |
| FND-003 | low | `fuzz_free_variant_over_arbitrary_candidates` (and `check_free_variant`) always pass `widen = None`, so the cross-machine path of `free_variant`, the only one where FND-002 occurs, is never fuzzed. | crates/relay-core/tests/names.rs:559-573, crates/relay-core/tests/names.rs:616-620 |
| FND-004 | low | Design note, not a correctness bug: the variant changes the name's first byte, which is the part humans read (`qsl-aaaaaaaaaaaaaaaaaaaaaaaaaa-2` becomes `asl-aaaaaaaaaaaaaaaaaaaaaaaaaa-2`, so the team prefix changes). Changing the last kept head byte instead would keep the prefix and stay valid, because the head is trimmed of `-`, so replacing its last byte with a different letter adds no dash. The trade-off is not stated in the doc comment. | crates/relay-core/src/naming.rs:204-215 |
| FND-005 | low | Rust idiom, style note: `('a'..='z').find(\|c\| *c != first).unwrap_or('a')` is a search for a two-way choice, `if first == 'a' { 'b' } else { 'a' }`. `chars.next().unwrap_or('a')` covers an unreachable case (the head is never empty: room >= 1 and a name starts with a letter). If it were ever reached, it would silently build `b-<suffix>` rather than fail a `debug_assert!`. No panic surface either way. | crates/relay-core/src/naming.rs:212-215 |

## Rust lane (rust-review)

- Panic surface: no `unwrap`/`expect` on a request path. The two `unwrap_or`
  calls are total. The byte slicing in `with_suffix` is ASCII-only (`slug_body`
  output and a validated `AgentName`), so `truncate(room)` cannot split a char.
  `from_constructed` has a debug-assert on validity. The sweep ran in debug, so
  every one of the ~52.7M constructions passed that assert.
- Integer bounds: `room = MAX_NAME_LEN - 1 - suffix.len()` with
  `suffix.len() <= 30`, so there is no underflow.
- No new `allow`, no `unsafe`, no new public item, no `.github` change. No
  proptest-regressions file was added. The fuzz config sets
  `failure_persistence: None`. The only tracked regressions file is the
  pre-existing `crates/agent-msg/proptest-regressions/stdin.txt` (from #24).
- Test conventions: new tests carry `Tracing:`/`ACs:` tags. `tc_022_01_numbered_*`
  silently `continue`s on an unparsable base (none occur today). That is
  acceptable, but a shrinking generator would go unnoticed.

## Verification performed (scratch copy, worktree target reused)

1. Reviewer sweep of the numbered path. Bases: every length 1 to 32 (40 random
   names per length, plus shapes with the first letter a/b/y/z, all-`a`,
   all-`z`, `-0…`, `_9…`, trailing `-`), and every `<prefix>-<n>` base for n = 1
   to 1000 at lengths 29 to 32 (every n; plus every 97th n at shorter lengths).
   The prefixes start with `a`, `b` or `z`, with an interior `-`, a trailing
   `_`, or a trailing `-`. For each base, every n from 1 to 1000 was checked:
   valid `AgentName` (parse), <=32, != base, ends in the full `-<n>`, starts
   lowercase, never `all`, no new `--`, pairwise distinct. `free_variant` with
   998 refusals still returns (no early NoFreeName). Result: 52,608 bases, 52.6M
   calls, 18,003 head-letter-path hits, 0 failures (316 s debug).
2. Reviewer sweep of the widen path: every suffix length 1 to 30 (`b`*k, `9`*k,
   `_`*k and 30 random `[az09_-]` strings per length, some with `--` and leading
   or trailing dashes), on bases of every length that end in `-<clean suffix>`
   (first letter a, z or m) plus random bases. Result: 121,035 calls, 11,493
   head-letter hits, every result valid, <=32, != base, ending in the full
   cleaned suffix, and no new `--`. The only repeats were FND-002
   (widen == numbered, 3,713).
3. The head-letter change cannot produce an invalid or reserved name: the head's
   first byte is the base's first byte (a letter), and the replacement is `a`
   or `b`. The result always contains `-`, so it is never `all`, and it never
   starts with `esc-`-style prefixes (`e` is never chosen).
4. The mutation tc_022_03 was run against real Redis (throwaway redis:7): it
   passes on the branch and fails on main's `naming.rs`
   (`qsl-aaaaaaaaaaaaaaaaaaaaaaaaaa-2 -> qsl-aaaaaaaaaaaaaaaaaaaaaaaaaa-3`).
5. The names.rs `tc_022` tests were also run with mutations. On main's
   `naming.rs`, 4 fail (grid, numbered, 31/32 regression, seed corpus). On
   round-1 `naming.rs` (b4d4197), 2 fail (grid, 31/32 regression): F-1 is caught.
   The seed corpus passes there, which is FND-001.
6. Fuzz: `make fuzz` (default 5,000 cases) ran both new lanes with the rest,
   and everything passed. `PROPTEST_CASES=100000` with `PROPTEST_RNG_SEED` 1 to 6
   on `fuzz_with_suffix_over_arbitrary_text` and
   `fuzz_free_variant_over_arbitrary_candidates`: all 6 seeds passed, with
   both lanes green on every seed (112 s to 159 s per seed, debug).
7. `make check-matrix` passed (290 rows).

## Coverage of the permanent sweep

In `tc_022_01_numbered_variants_are_valid_different_and_distinct_for_every_base_shape`,
only the bases numbered 2, 10, 100, 999 and 1000 get every n. The other
`<prefix>-<n>` bases get n from 1 to 12, their own n, 999 and 1000. The
alphabet bases get 1 to 40, 99, 100, 999 and 1000. Judgment: this is adequate.
The rebuild path fires only for `with_suffix(base, n)` where the base is 31 or
32 bytes and ends in `-<n>`. Each `<prefix>-<n>` base is always tested with
its own n, so every rebuild trigger in that family is exercised. Distinctness
across n follows structurally: every result ends in `-<n>` and n has no dash.
The reviewer's full sweep (every n for every base, above) found nothing that
the narrower sweep misses. A full version would give every base in the PR's own
base set every n from 1 to 1000. That is 32,629 bases: 20,000 `<prefix>-<n>`
bases, of which all but 20 would gain about 985 n each, and 12,629 alphabet
and tail bases, gaining about 956 n each. In total, that adds about 31.8M
`with_suffix` calls. The reviewer ran this full version in debug on the
scratch copy: 32,629 bases, 0 failures, 213 s. That ran alongside a fuzz run;
the coder measured 83 s. It is not needed as a permanent lane. If wanted, it
belongs as an `#[ignore]`d `fuzz_`-prefixed test, so `make fuzz` picks it up,
and never in `make test`, where it would roughly double the relay-core test
time.

## Earlier findings (round 1, PR comment 5847657076) re-verified at b101903

| Earlier | Status at b101903 | Evidence |
|---|---|---|
| F-1 (Medium) suffix tail cut on the widen path | fixed | The suffix-tail branch is gone. The widen sweep (121,035 calls) shows the full cleaned suffix every time. Round-1 `naming.rs` fails the grid and regression tests. |
| F-2 (Low) 32-byte/full-length wording | fixed | Comment at naming.rs:205-206 and the test name `tc_022_01_a_variant_never_equals_a_31_or_32_byte_base_ending_in_the_suffix` say 31 or 32. |
| F-3 (Low) no resolver-level AC-3 test | fixed | `tc_022_03_a_candidate_ending_in_a_number_still_gets_the_dash_two_variant`: passes on the branch, fails on main (real Redis). |
| F-4 (Low) grid could not reach the bug | fixed | The grid adds bases ending in the suffix at lengths 28 to 32 and fails on main. |
| F-5 (Low) no fuzz lane for with_suffix/free_variant | fixed, with gaps | Two lanes were added and run under `make fuzz`. Weaknesses: FND-001 and FND-003. |
| F-6 (Info) no CI checks | unchanged | `gh pr checks 29`: no checks reported. The lead runs `make ci` separately. |

## Verdict

**PASS (merge recommended)**. The round-2 fix is correct over every sweep the
reviewer ran. The five findings are low: FND-001 and FND-003 are test-strength
gaps, FND-002 is a doc overstatement of a harmless, largely pre-existing
duplicate ask, and FND-004 and FND-005 are design and style notes. None needs
to block the merge. They can be fixed in this PR or deferred.

## Dispositions

Disposition pass 1, reviewed at `ea19b51a8f53d6ca692547089be97efd3a67b749`
(fix commit `ea19b51`, the only commit in `b101903..ea19b51`).

| FND | outcome | sha/reason |
|-----|---------|------------|
| FND-001 | fixed ea19b51 | `check_with_suffix` now asserts `s.ends_with("-" + cleaned_suffix(suffix))`, with `cleaned_suffix` written from the requirement, not from `with_suffix`. Mutation: with b4d4197's `naming.rs` the seed corpus fails on the tail (`"a-bbbbbbbbbbbbbbbbbbbbbbbbbbbbb" must end with "-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"`) and `fuzz_with_suffix_over_arbitrary_text` failed in 5 of 6 runs at 5,000 cases and 1 of 1 at 100,000 cases (shrinks to base `a` plus a 29 to 30 byte suffix). |
| FND-002 | fixed ea19b51 | The false doc claim is replaced by an accurate one (distinct unless one cleaned suffix is a dash-suffix of the other), and `free_variant` skips a numbered variant equal to the widened name. Reviewer widen sweep: 976 widen inputs hit the duplicate; each asked 999 distinct names, the set equal to {widened} plus the numbered 2 to 1000, and the last name in the sequence was still returned when it was the only free one. |
| FND-003 | fixed ea19b51 | `fuzz_free_variant_over_arbitrary_candidates` now draws `widen` from `option::of(...)` including the `[0-9]-[0-9]` shape and asserts no name is asked twice. With b101903's `naming.rs` the lane failed 3 of 3 runs at 5,000 cases ("a name was asked twice"). |
| FND-004 | fixed ea19b51 | The rebuild path now replaces the last head byte (next to the dash), keeping the prefix; the doc comment states it. `tc_022_03` asserts the `qsl-` prefix survives (passes against real Redis). |
| FND-005 | fixed ea19b51 | The `find`/`unwrap_or` pair is gone: `if last == "a" { 'b' } else { 'a' }` over `head.split_at(head.len().saturating_sub(1))`. The unreachable empty-head case would now build `a-<suffix>`, still valid; no `debug_assert!` was added, which is acceptable because the output stays a valid name. |

### Verification performed at ea19b51 (scratch copy, worktree target reused)

1. Reviewer numbered sweep, own oracle (validity re-implemented from
   `name.rs`, not `AgentName::parse` alone): 271,667 bases (every length 1 to
   32 with first byte a/b/z/m, random bodies and trailing `-`/`_`/digit/`a`/`b`
   shapes; every `<prefix>-<n>` base at lengths 28 to 32 for n = 1 to 1000,
   prefixes starting a/b/z, last prefix byte a/b/z/`_`/`0`/`-`, plain, with
   `-` or `_` at index 1), each with every n from 1 to 1000: 271,667,000
   calls, 90,001 rebuild-path hits, 0 failures (3,949 s, debug, so the
   `from_constructed` debug-assert ran on every call). Checked: valid, <=32,
   != base, ends in the whole `-<n>`, never `all`, no new `--`, distinct
   across n, and on the rebuild path the result differs from the base in
   exactly one byte, the one next to the dash, replaced by `a` (or `b` when it
   was `a`). `free_variant` (no widen) on every 37th base: 999 distinct asks,
   no early `None`, last name reached.
2. Reviewer widen sweep: suffixes 1 to 30 bytes (`a`, `b`, `9`, `_` runs, 30
   random `[az09_-]` strings per length, `h..-9` and `0..-12` machine shapes)
   on bases of every length ending in the cleaned suffix (1-byte heads
   included) plus random bases: 550,907 calls, 34,096 rebuild hits, 0
   failures. 3,584 widen results equal a numbered variant; every one is the
   dash-suffix case the new doc names (a spot check over all n found no
   other). `free_variant` with widening: 976 duplicate cases, all 999 distinct
   asks, sets exact.
3. Analysis: the rebuild path fires only when `head` is a non-empty prefix of
   the base that starts with a letter; the replaced byte becomes a letter, so
   no `--` can appear and a 1-byte head stays a letter (`b-<30 bytes>` becomes
   `a-...`). Numbered variants stay distinct because n has no dash, so the
   last dash segment identifies n.
4. `tc_022_03_*` and the rest of `relay-ops/tests/collision.rs` pass against a
   throwaway redis:7. `relay-core --test names` passes at the head: 16 passed,
   5 ignored, 14.8 s (19.3 s wall with the rebuild); acceptable for `make test`.
5. `fuzz_with_suffix_every_n_for_every_base_shape` is `#[ignore]`d with the
   `fuzz_` prefix, so `make fuzz` (`--ignored fuzz_`) runs it and `make test`
   does not.
6. No `proptest-regressions` file added (only the pre-existing
   `crates/agent-msg/proptest-regressions/stdin.txt`), no `.github` change, no
   new `allow`, and no `unwrap`/`expect` outside tests.

## New findings (disposition pass 1)

| ID | Severity | Summary | Refs |
|----|----------|---------|------|
| FND-006 | low | The seed corpus's duplicate case, `check_free_variant(&dup, Some("0-9"), 0)`, refuses nothing, so `free_variant` returns the widened name on the first ask and never reaches the numbered loop where the duplicate would be asked. The same holds for the per-corpus `for widen in [None, Some(suffix)]` calls at `refuse = 0`. Mutation: with b101903's `naming.rs` (no skip) the seed-corpus test passes. The dedicated `tc_022_01_free_variant_asks_each_name_once_when_the_widened_name_is_a_numbered_variant` and the fuzz lane do catch it, so this is a redundant, vacuous check, not a coverage hole. A `refuse` of `MAX_VARIANT_NUMBER` (or 9) would make it bite. | crates/relay-core/tests/names.rs:646-653 |
