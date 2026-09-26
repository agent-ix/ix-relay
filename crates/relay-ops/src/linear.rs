//! The slice of the `linear` CLI `resolve` and `escalation post-linear` need,
//! behind a trait.
//!
//! [`RealLinear`] shells out to `linear issue comment add <ID> --body-file
//! <path>`, same as a human would from the terminal, through
//! [`herdr_cli::ProcessRunner`] — the same run-with-a-deadline-and-kill
//! plumbing [`crate::herdr::RealHerdr`] uses, so a hung `linear` process
//! (a stuck auth prompt, a dead network) is killed and reported as a
//! [`herdr_cli::ProcessError::Timeout`] instead of blocking whichever thread
//! called `comment` forever. A fake implementing [`Linear`] lets that
//! command wiring be unit-tested without a real Linear workspace.
//! [`RealLinear`] is itself generic over [`herdr_cli::ProcessRunner`] so its
//! own argv-building can be tested without spawning `linear` for real (see
//! this module's `tests`).

use std::time::Duration;

use herdr_cli::{ProcessError, ProcessRunner};

/// How long one `linear issue comment add` call waits before it's killed.
/// `linear` talks to a real API over the network; long enough to survive a
/// slow connection, short enough that a hung/blocked call (e.g. an
/// interactive auth prompt neither `resolve` nor `post-linear` can answer)
/// doesn't wedge the caller.
const LINEAR_TIMEOUT: Duration = Duration::from_secs(30);

/// Something went wrong posting a comment through the `linear` CLI.
#[derive(Debug, thiserror::Error)]
pub enum LinearError {
    /// Couldn't write the comment body to the temp file `--body-file` reads.
    #[error("could not write comment body: {0}")]
    Io(String),
    /// The `linear` binary itself could not be run, or the call had to be
    /// killed after its own deadline (30s, this module's own
    /// `LINEAR_TIMEOUT`).
    #[error(transparent)]
    Process(#[from] ProcessError),
    /// `linear` exited nonzero.
    #[error("linear exited with status {status}: {stderr}")]
    Failed {
        /// The process exit code, or -1 if it was killed by a signal.
        status: i32,
        /// Raw stderr.
        stderr: String,
    },
}

/// Posting a comment to a ticket.
pub trait Linear {
    /// Posts `body` (Markdown) as a comment on `ticket` (a Linear key, e.g.
    /// `AGE-2014`).
    fn comment(&self, ticket: &str, body: &str) -> Result<(), LinearError>;
}

/// Shells out to the real `linear` CLI on `PATH`, through `R`
/// ([`herdr_cli::RealProcessRunner`] in production; a recording fake in
/// this module's own tests, to check the exact argv this type builds).
pub struct RealLinear<R: ProcessRunner = herdr_cli::RealProcessRunner> {
    runner: R,
}

impl RealLinear<herdr_cli::RealProcessRunner> {
    /// The production `RealLinear`, spawning real `linear` calls.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runner: herdr_cli::RealProcessRunner,
        }
    }
}

impl Default for RealLinear<herdr_cli::RealProcessRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: ProcessRunner> Linear for RealLinear<R> {
    fn comment(&self, ticket: &str, body: &str) -> Result<(), LinearError> {
        let mut file = tempfile::Builder::new()
            .prefix("ix-relay-esc-comment-")
            .suffix(".md")
            .tempfile()
            .map_err(|e| LinearError::Io(e.to_string()))?;
        std::io::Write::write_all(&mut file, body.as_bytes())
            .map_err(|e| LinearError::Io(e.to_string()))?;
        // `tempfile::NamedTempFile` deletes itself on drop, including on the
        // early returns below — no separate cleanup step to forget, unlike a
        // hand-rolled `std::env::temp_dir()` path.
        let path = file.path();
        let path_str = path.to_string_lossy();
        let args = ["issue", "comment", "add", ticket, "--body-file", &path_str];
        let out = self.runner.run("linear", &args, LINEAR_TIMEOUT)?;
        if out.success {
            Ok(())
        } else {
            Err(LinearError::Failed {
                status: out.status_code,
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use herdr_cli::Output;

    use super::*;

    struct RecordingLinear {
        calls: std::cell::RefCell<Vec<(String, String)>>,
        fail: bool,
    }

    impl Linear for RecordingLinear {
        fn comment(&self, ticket: &str, body: &str) -> Result<(), LinearError> {
            self.calls
                .borrow_mut()
                .push((ticket.to_owned(), body.to_owned()));
            if self.fail {
                Err(LinearError::Failed {
                    status: 1,
                    stderr: "not logged in".into(),
                })
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn fake_records_the_call_and_can_be_made_to_fail() {
        let ok = RecordingLinear {
            calls: std::cell::RefCell::default(),
            fail: false,
        };
        ok.comment("AGE-2014", "body").unwrap();
        assert_eq!(
            ok.calls.borrow()[0],
            ("AGE-2014".to_owned(), "body".to_owned())
        );

        let failing = RecordingLinear {
            calls: std::cell::RefCell::default(),
            fail: true,
        };
        assert!(failing.comment("AGE-2014", "body").is_err());
    }

    struct RecordingRunner {
        calls: RefCell<Vec<(String, Vec<String>, Duration)>>,
        /// The `--body-file` path's contents, read *during* the fake `run`
        /// call, while `RealLinear`'s `NamedTempFile` is still alive (it
        /// deletes itself as soon as `comment` returns) — not a path a test
        /// could still read afterward.
        seen_body_file_contents: RefCell<Option<String>>,
        response: Output,
    }

    impl ProcessRunner for RecordingRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            timeout: Duration,
        ) -> Result<Output, ProcessError> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.iter().map(|s| (*s).to_owned()).collect(),
                timeout,
            ));
            if let Some(pos) = args.iter().position(|a| *a == "--body-file")
                && let Some(path) = args.get(pos + 1)
            {
                *self.seen_body_file_contents.borrow_mut() = Some(
                    std::fs::read_to_string(path).expect("body file readable while run() executes"),
                );
            }
            Ok(self.response.clone())
        }
    }

    fn ok_output() -> Output {
        Output {
            status_code: 0,
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn real_linear_sends_exactly_the_documented_argv_a_readable_body_file_and_the_deadline() {
        let runner = RecordingRunner {
            calls: RefCell::default(),
            seen_body_file_contents: RefCell::default(),
            response: ok_output(),
        };
        let linear = RealLinear { runner };
        linear.comment("AGE-2014", "the comment body").unwrap();

        let calls = linear.runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "linear");
        assert_eq!(calls[0].1[0], "issue");
        assert_eq!(calls[0].1[1], "comment");
        assert_eq!(calls[0].1[2], "add");
        assert_eq!(calls[0].1[3], "AGE-2014");
        assert_eq!(calls[0].1[4], "--body-file");
        assert_eq!(
            calls[0].2, LINEAR_TIMEOUT,
            "a bounded deadline, not indefinite"
        );

        let contents = linear
            .runner
            .seen_body_file_contents
            .borrow()
            .clone()
            .unwrap();
        assert_eq!(contents, "the comment body");
    }

    #[test]
    fn real_linear_surfaces_a_nonzero_exit_as_failed() {
        let runner = RecordingRunner {
            calls: RefCell::default(),
            seen_body_file_contents: RefCell::default(),
            response: Output {
                status_code: 1,
                success: false,
                stdout: Vec::new(),
                stderr: b"not logged in".to_vec(),
            },
        };
        let linear = RealLinear { runner };
        let err = linear.comment("AGE-2014", "body").unwrap_err();
        assert!(
            matches!(&err, LinearError::Failed { stderr, .. } if stderr.contains("not logged in"))
        );
    }

    /// A hung `linear` process is killed and reported as a timeout, not left
    /// to block the caller forever — the whole point of routing this
    /// through `herdr_cli::ProcessRunner` instead of a bare blocking
    /// `Command::output()`.
    #[test]
    fn a_hung_linear_call_is_killed_and_reported_as_a_timeout() {
        struct HangingRunner;
        impl ProcessRunner for HangingRunner {
            fn run(
                &self,
                program: &str,
                args: &[&str],
                timeout: Duration,
            ) -> Result<Output, ProcessError> {
                herdr_cli::RealProcessRunner.run(program, args, timeout)
            }
        }
        let linear = RealLinear {
            runner: TimeoutOverride {
                inner: HangingRunner,
                timeout: Duration::from_millis(200),
            },
        };
        let err = linear.comment("AGE-2014", "body").unwrap_err();
        assert!(
            matches!(&err, LinearError::Process(ProcessError::Timeout { .. })),
            "{err:?}"
        );
    }

    /// Overrides whatever deadline the caller asked for with a short one, so
    /// the timeout test above doesn't have to wait out the real 30s
    /// [`LINEAR_TIMEOUT`] — it still exercises `RealProcessRunner`'s actual
    /// spawn/poll/kill path via `sleep`, standing in for a hung `linear`.
    struct TimeoutOverride<R> {
        inner: R,
        timeout: Duration,
    }

    impl<R: ProcessRunner> ProcessRunner for TimeoutOverride<R> {
        fn run(
            &self,
            _program: &str,
            _args: &[&str],
            _timeout: Duration,
        ) -> Result<Output, ProcessError> {
            self.inner.run("sleep", &["5"], self.timeout)
        }
    }
}
