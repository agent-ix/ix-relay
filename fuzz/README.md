# Fuzz lanes

The `fuzz_*` tests are **property-based fuzz lanes**, built on `proptest`. They are
not coverage-guided fuzzing. `cargo-fuzz` and `libFuzzer` need a nightly
toolchain, which this repository does not pin, so each Fuzz-kind obligation from
`quoin advise` is implemented as a `proptest` test over arbitrary bytes and
adversarial strings, with structure-aware generators where the input has a
grammar.

## Running

```bash
make fuzz                    # every fuzz_ test, 5,000 cases each, against a throwaway redis:7
make fuzz FUZZ_CASES=100000  # more cases (the hook lane spawns a process per case, so it is slow)
PROPTEST_CASES=200000 cargo test -p relay-core --test names -- --ignored fuzz_   # one lane, no Redis
```

`PROPTEST_CASES` is proptest's own knob; `make fuzz` sets it from `FUZZ_CASES`. A
plain `make test` skips these tests (they are `#[ignore = "fuzz lane: ..."]`); the
Redis-backed ones also skip cleanly without `IX_RELAY_TEST_URL`.

## The lanes

| Obligation | Entry point | Test |
|---|---|---|
| FR-001-AC-1 (address parser) | `Address::from_str` | `relay-core/tests/address_resolution.rs`, `fuzz_address_parse_*` |
| FR-021-AC-1 (slugify) | `naming::slugify` | `relay-core/tests/names.rs`, `fuzz_slugify_*` |
| FR-022-AC-1 (variants) | `naming::with_suffix`, `naming::free_variant` | `relay-core/tests/names.rs`, `fuzz_with_suffix_*`, `fuzz_free_variant_*` |
| FR-026-AC-6 (config file) | `config::load_file` | `relay-core/src/config/evidence.rs`, `fuzz_config_file_*` |
| FR-008-AC-4 (registry decoding) | `Relay::agents` | `relay-core/tests/registry_decoding.rs`, `fuzz_registry_read_*` |
| FR-017-AC-1 (hook stdin) | `agent-msg hook claude` | `agent-msg/tests/hook_fuzz.rs`, `fuzz_hook_stdin_*` |
| FR-016-AC-1/2 (rendering) | `Message::render_from` | `relay-core/tests/text_render.rs`, `fuzz_render_*` |

Each lane asserts real properties (a valid result for every input, round trips
where defined, no panic, one-line errors that name the file, bounded output size,
and for the hook a per-run time bound), and each has a small **seed corpus** of
known-nasty inputs that runs in every `cargo test`. The address parser and the
renderer also run their generated (not raised-count) properties in every
`cargo test`.

## What real cargo-fuzz would add

- **Coverage guidance.** libFuzzer mutates towards unexplored branches; `proptest`
  samples from a generator, so an input that needs a specific byte sequence
  (a checksum, a magic prefix) is found only if the generator can produce it.
- **Corpus growth and minimisation** across runs, stored and shared.
- **Sanitizers** (ASan, UBSan) on the fuzzed code.
- **Long runs** (hours) with crash deduplication.

Writing these lanes discharges nothing on its own beyond what they assert when
run. The obligations stay `Fuzz`-recommended until a coverage-guided run is
recorded (`spec/tests.md` says so on each row).
