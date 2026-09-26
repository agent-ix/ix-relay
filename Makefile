# =============================================================================
# ix-relay Makefile
# =============================================================================

CARGO ?= cargo

.PHONY: help
help:
	@echo "Available targets:"
	@echo "  make fmt              - Format with rustfmt"
	@echo "  make fmt-check        - Verify formatting (CI gate)"
	@echo "  make lint             - Clippy with -D warnings"
	@echo "  make test             - cargo test"
	@echo "  make build            - Release build"
	@echo "  make clean            - cargo clean"
	@echo "  make deny             - cargo deny check licenses"
	@echo "  make audit-unsafe     - Enforce // SAFETY: comments on unsafe blocks"
	@echo "  make docs             - cargo doc, denying missing/broken doc links"
	@echo "  make fuzz             - Property-based fuzz lanes at a raised case count (see fuzz/README.md)"
	@echo "  make check-matrix     - Recount spec/tests.md against its summary tables (needs python3)"
	@echo "  make ci               - All CI gates locally (fmt-check + lint + test-redis + deny + audit-unsafe + docs + check-matrix)"

# =============================================================================
# Format / Lint / Test
# =============================================================================

.PHONY: fmt
fmt:
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check:
	$(CARGO) fmt --all -- --check

.PHONY: lint
lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

.PHONY: test
test:
	$(CARGO) test --workspace

# Same tests, plus the Redis integration tests, against a throwaway redis:7 container.
.PHONY: test-redis
test-redis:
	bash scripts/test-with-redis.sh

# The `fuzz_*` tests are proptest-driven fuzz lanes (arbitrary bytes and
# adversarial strings, raised case counts). They are `#[ignore]`d in a plain
# `make test`; this runs only them, against a throwaway redis:7 so the
# Redis-backed ones run too. FUZZ_CASES sets proptest's case count per test.
# See fuzz/README.md for what this is and what coverage-guided fuzzing adds.
FUZZ_CASES ?= 5000

.PHONY: fuzz
fuzz:
	PROPTEST_CASES=$(FUZZ_CASES) bash scripts/test-with-redis.sh -- --ignored fuzz_

.PHONY: build
build:
	$(CARGO) build --release

.PHONY: clean
clean:
	$(CARGO) clean

# =============================================================================
# Supply chain & safety
# =============================================================================

.PHONY: deny
deny:
	$(CARGO) deny check licenses

.PHONY: cargo-audit
cargo-audit:
	$(CARGO) audit

.PHONY: audit-unsafe
audit-unsafe:
	bash scripts/check_unsafe_comments.sh

# =============================================================================
# Documentation
# =============================================================================

# cargo doc is a separate lint pass from clippy: rustdoc-only lints such as
# rustdoc::broken_intra_doc_links only fire here, never under `make lint`.
.PHONY: docs
docs:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps --all-features

# The test matrix's summary tables must reconcile with its rows.
.PHONY: check-matrix
check-matrix:
	python3 scripts/check_test_matrix.py

# =============================================================================
# Composite
# =============================================================================

.PHONY: ci
ci: fmt-check lint test-redis deny audit-unsafe docs check-matrix
