//! `agent-msg doctor`: is the relay healthy, and if not, what is wrong and what to
//! do (FR-029).
//!
//! Read-only and side-effect free: it pops nothing, writes nothing (no
//! registration, no spool, no state file) and runs no command that changes state.
//! It works with Redis down, and says so. The pieces:
//!
//! - [`facts`]: gathers what there is to look at, as plain data;
//! - [`checks`]: turns the facts into [`report::Check`]s, each with a stable code;
//! - [`report`]: the typed report, printed as text or with `--json`.
//!
//! The exit status is 1 when any check is a `fail`. A `warn` is reported and
//! exits 0 unless `--strict`; an `unknown` (a check that could not tell) never
//! fails it. Which code is which is in `checks.rs` and FR-029.

mod checks;
mod facts;
mod report;

use std::{
    io::{self, Write},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) use facts::{Env, Probes, RealProbes};
use relay_core::config::{self, CoreFlags};

use crate::spool::Spool;

/// Runs `doctor` with the command-line `flags` and prints to stdout.
pub(crate) fn run(flags: CoreFlags, json: bool, strict: bool) -> ExitCode {
    let (env, config_error) = match config::load_core(flags) {
        Ok(settings) => (
            env_from(
                settings.machine.value,
                settings.url.value,
                settings.prefix.value,
            ),
            None,
        ),
        Err(e) => match relay_core::machine_name() {
            Ok(machine) => (
                env_from(
                    machine,
                    config::DEFAULT_URL.to_owned(),
                    relay_core::DEFAULT_PREFIX.to_owned(),
                ),
                Some(e.to_string()),
            ),
            // Neither the config nor the hostname gives a machine name: there
            // is no machine to report on.
            Err(name_error) => {
                eprintln!("agent-msg doctor: {e}; {name_error}");
                return ExitCode::FAILURE;
            }
        },
    };
    let mut out = io::stdout().lock();
    match run_with(
        &env,
        config_error,
        &RealProbes,
        now_ms(),
        (json, strict),
        &mut out,
    ) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("agent-msg doctor: {e}");
            ExitCode::FAILURE
        }
    }
}

fn env_from(machine: relay_core::MachineName, url: String, prefix: String) -> Env {
    Env {
        machine,
        url,
        prefix,
        state_file: config::state_dir().map(|d| d.join(relay_core::health::WATCHER_FILE_NAME)),
        spool_file: Spool::in_config_dir().map(|s| s.path().to_path_buf()),
        redis_budget: facts::REDIS_BUDGET,
        // A config that does not load is reported as such; the mode is then the
        // default, which reads no socket status.
        delivery: config::watcher_delivery(),
        socket_status_dir: config::state_dir().map(|d| relay_core::socket_status::dir_in(&d)),
    }
}

/// Gathers, checks and prints. Returns whether nothing failed.
pub(crate) fn run_with(
    env: &Env,
    config_error: Option<String>,
    probes: &dyn Probes,
    now_ms: i64,
    (json, strict): (bool, bool),
    out: &mut impl Write,
) -> io::Result<bool> {
    let facts = facts::gather(env, config_error, probes, now_ms);
    let report = report::Report::new(checks::evaluate(&facts), strict);
    let written = if json {
        report.write_json(out)
    } else {
        report.write_text(out)
    };
    match written {
        Ok(()) => Ok(report.ok),
        // `doctor | head` closes the pipe: the exit status is still the relay's
        // health, not a failure of the reader.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(report.ok),
        Err(e) => Err(e),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
