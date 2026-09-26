//! The one herdr operation `relay-tui` needs that `relay-ops`'s own `Herdr`
//! trait doesn't: a desktop notification for a newly arrived escalation
//! (`--notify`). Kept as its own tiny trait/impl pair rather than added to
//! `relay_ops::herdr::Herdr` — that trait already documents itself as
//! "only the commands \[its crate\] actually calls" (investigate's
//! tab/agent lifecycle); a notification is `relay-tui`-only, same as
//! `relay-watcher` has its own `Herdr::notify` for the same herdr
//! subcommand, used for a different reason (a blocked lead, not a new
//! escalation).

use std::time::Duration;

use herdr_cli::{ProcessError, ProcessRunner};

/// How long to wait for one `herdr notification show` call.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// Shows a desktop notification.
pub trait Notifier {
    /// `herdr notification show <title> --body <body>`.
    fn notify(&self, title: &str, body: &str) -> Result<(), ProcessError>;
}

/// Shells out to the real `herdr` binary on `PATH`, through `R`
/// ([`herdr_cli::RealProcessRunner`] in production; a recording fake in
/// this module's own tests).
pub struct RealNotifier<R: ProcessRunner = herdr_cli::RealProcessRunner> {
    runner: R,
}

impl RealNotifier<herdr_cli::RealProcessRunner> {
    /// The production `RealNotifier`, spawning real `herdr` calls.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runner: herdr_cli::RealProcessRunner,
        }
    }
}

impl Default for RealNotifier<herdr_cli::RealProcessRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: ProcessRunner> Notifier for RealNotifier<R> {
    fn notify(&self, title: &str, body: &str) -> Result<(), ProcessError> {
        herdr_cli::call::<serde_json::Value>(
            &self.runner,
            "herdr",
            &["notification", "show", title, "--body", body],
            NOTIFY_TIMEOUT,
        )
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use herdr_cli::Output;

    use super::*;

    struct RecordingRunner {
        calls: RefCell<Vec<(String, Vec<String>)>>,
    }

    impl ProcessRunner for RecordingRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            _timeout: Duration,
        ) -> Result<Output, ProcessError> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.iter().map(|s| (*s).to_owned()).collect(),
            ));
            Ok(Output {
                status_code: 0,
                success: true,
                stdout: br#"{"result":{}}"#.to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn notify_sends_exactly_the_documented_argv() {
        let runner = RecordingRunner {
            calls: RefCell::default(),
        };
        let notifier = RealNotifier { runner };
        notifier
            .notify("ix-relay: AGE-2015", "escalation e7 from qsl-lead")
            .unwrap();
        assert_eq!(
            notifier.runner.calls.borrow()[0].1,
            vec![
                "notification",
                "show",
                "ix-relay: AGE-2015",
                "--body",
                "escalation e7 from qsl-lead"
            ]
        );
    }
}
