//! Shared test-only env var isolation for `env.rs` and `config.rs`.
//!
//! Both modules' tests mutate process-global env vars (`std::env::set_var` is
//! not per-thread), and `cargo test` runs tests in the same binary on
//! separate threads by default. Two modules each keeping their own lock let
//! their tests interleave and flake — this is the one lock and the one guard
//! every test that touches these vars must use.

#![expect(
    unsafe_code,
    reason = "edition 2024 makes std::env::set_var/remove_var unsafe; calls are serialised by ENV_LOCK, so they race only with tests that do not take it"
)]

use std::sync::Mutex;

/// Every env var any test in this crate mutates. A guard clears all of them
/// on construction (so a value left in the shell that started `cargo test` —
/// a lead session's own `IX_RELAY_URL`, say — can't leak into a test that
/// didn't ask for it) and again on drop (so one test can't leak into the
/// next).
pub(crate) const ALL_ENV_VARS: &[&str] = &[
    "IX_RELAY_CONFIG",
    "IX_RELAY_URL",
    "IX_RELAY_PREFIX",
    "IX_RELAY_MACHINE",
    "IX_RELAY_HUMAN",
    "IX_RELAY_WATCHER_INTERVAL_SECS",
    "IX_RELAY_WATCHER_OTLP_INTERVAL_SECS",
    "IX_RELAY_WATCHER_OTLP_ENDPOINT",
    "IX_RELAY_WATCHER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_PROTOCOL",
    "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_RESOURCE_ATTRIBUTES",
    "OTEL_SERVICE_NAME",
];

/// Serializes every test that touches [`ALL_ENV_VARS`].
pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Clears every var in [`ALL_ENV_VARS`], then sets `overrides`. Restores a
/// clean (all-cleared) state on drop. Construct only while holding
/// [`ENV_LOCK`].
pub(crate) struct EnvGuard;

impl EnvGuard {
    /// An isolated environment with only `overrides` set.
    pub(crate) fn isolated(overrides: &[(&'static str, &str)]) -> Self {
        for name in ALL_ENV_VARS {
            // SAFETY: single-threaded w.r.t. these env vars, guarded by ENV_LOCK.
            unsafe { std::env::remove_var(name) };
        }
        for (name, value) in overrides {
            // SAFETY: single-threaded w.r.t. these env vars, guarded by ENV_LOCK.
            unsafe { std::env::set_var(name, value) };
        }
        Self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for name in ALL_ENV_VARS {
            // SAFETY: single-threaded w.r.t. these env vars, guarded by ENV_LOCK.
            unsafe { std::env::remove_var(name) };
        }
    }
}
