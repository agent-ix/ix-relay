//! Shared plumbing for shelling out to the `herdr` CLI.
//!
//! Both `relay-watcher` and `agent-msg` need to run `herdr <args>`, decode its
//! documented `{"result": ...}` / `{"error": {"code", "message"}}` JSON
//! envelope, and turn a hung or crashed call into a typed error rather than
//! wedging the caller. Before this crate existed both binaries hand-rolled
//! that logic — `relay-watcher`'s copy polled for exit with a timeout and
//! kill; `agent-msg`'s copy used a plain blocking `Command::output()` with no
//! timeout at all, so a hung `herdr agent start` could block `investigate`
//! forever. One copy here, with a timeout every caller gets by construction.
//!
//! This crate has no opinion on which herdr subcommands exist or what they
//! mean — each binary keeps its own `Herdr` trait naming only the commands it
//! actually calls (`agent get`/`prompt`/`notification show` for
//! `relay-watcher`; `tab create`/`tab close`/`agent start` for `agent-msg`)
//! and its own fakes for testing that trait's callers. What lives here is
//! only the plumbing underneath: running a process and decoding its output.
//!
//! [`ProcessRunner`] is the seam: [`RealProcessRunner`] actually spawns a
//! child process, and a test double can record the exact program and
//! arguments a caller built and return a canned [`Output`] — so a caller's
//! own argv-building logic (e.g. `agent-msg`'s `RealHerdr::tab_create`) is
//! testable without ever spawning `herdr` for real.

use std::{
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

/// Something went wrong asking herdr (or, via [`ProcessRunner`], any external
/// CLI following the same run/decode shape) to do something.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProcessError {
    /// The binary itself could not be run (not on `PATH`, ...). No input can
    /// have reached whatever it was about to control.
    #[error("could not run {program}: {reason}")]
    Spawn {
        /// The program that failed to start.
        program: String,
        /// Why, per [`std::io::Error`]'s own message.
        reason: String,
    },
    /// herdr's own reported error, with its stable code, parsed from stderr
    /// (herdr writes `{"error":{"code":...,"message":...}}` there and exits
    /// nonzero) or stdout for a rare success-exit error envelope.
    #[error("herdr: {code}: {message}")]
    Reported {
        /// herdr's stable error code, e.g. `agent_blocked`.
        code: String,
        /// The human-readable message that came with it.
        message: String,
    },
    /// The process exited nonzero but its stderr wasn't the documented JSON
    /// error envelope (a crash, a version mismatch, a plain clap usage error, ...).
    #[error("{program} exited with status {status}: {stderr}")]
    UnexpectedExit {
        /// The program that was run.
        program: String,
        /// The process exit code, or -1 if it was killed by a signal.
        status: i32,
        /// Raw stderr.
        stderr: String,
    },
    /// The process exited successfully but stdout wasn't the documented JSON
    /// envelope, or not the shape the caller expected.
    #[error("could not parse {program} output: {reason}")]
    BadOutput {
        /// The program that was run.
        program: String,
        /// The decode failure.
        reason: String,
    },
    /// The caller gave up waiting and killed the process. Whether input
    /// reached whatever it was about to control before the kill is unknown.
    #[error("{program} call timed out after {secs}s and was killed")]
    Timeout {
        /// The program that was run.
        program: String,
        /// The timeout that was exceeded.
        secs: u64,
    },
}

impl ProcessError {
    /// Whether herdr's own documented contract guarantees nothing reached the
    /// agent when this error occurred — safe for a caller that already popped
    /// something (e.g. `relay-watcher` popping a batch before `agent prompt`)
    /// to requeue it. Conservative: anything not explicitly documented that
    /// way (including a bare nonzero exit, or a timeout) is ambiguous, never
    /// requeued.
    #[must_use]
    pub fn definitely_not_sent(&self) -> bool {
        match self {
            Self::Spawn { .. } => true,
            Self::Reported { code, .. } => {
                // herdr's own CLI docs: "rejects an agent already waiting at
                // an approval or question dialog with agent_blocked before
                // sending any input." `agent_not_found` means there was no
                // agent in the pane to send input to at all.
                matches!(code.as_str(), "agent_blocked" | "agent_not_found")
            }
            Self::UnexpectedExit { .. } | Self::BadOutput { .. } | Self::Timeout { .. } => false,
        }
    }
}

/// What running a child process to completion produced.
#[derive(Debug, Clone)]
pub struct Output {
    /// The process exit code, or -1 if it was killed by a signal.
    pub status_code: i32,
    /// Whether it exited successfully (status 0).
    pub success: bool,
    /// Captured stdout.
    pub stdout: Vec<u8>,
    /// Captured stderr.
    pub stderr: Vec<u8>,
}

/// Runs one external command to completion. The seam [`RealProcessRunner`]
/// implements for real, and a test double implements to record the exact
/// argv a caller built without spawning anything.
pub trait ProcessRunner {
    /// Runs `program <args>`, killing it if it hasn't exited within `timeout`.
    ///
    /// # Errors
    ///
    /// [`ProcessError::Spawn`] if the process couldn't start at all;
    /// [`ProcessError::Timeout`] if it had to be killed.
    fn run(&self, program: &str, args: &[&str], timeout: Duration) -> Result<Output, ProcessError>;
}

/// Most bytes kept from each of a child's stdout and stderr. herdr's JSON
/// replies are a few hundred bytes; anything past this is read and discarded
/// (so the child is never blocked on a full pipe) rather than buffered
/// without bound.
const MAX_CAPTURE_BYTES: u64 = 8 * 1024 * 1024;

/// Read currently available bytes without waiting for descendants to close a
/// pipe. Bound work per visit so a continuously chatty child cannot prevent
/// deadline checks or starve its other output stream.
fn capture_available(reader: &mut impl Read, kept: &mut Vec<u8>) -> std::io::Result<()> {
    let mut buffer = [0u8; 65536];
    for _ in 0..32 {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let available = (MAX_CAPTURE_BYTES as usize).saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..count.min(available)]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn nonblocking(pipe: &impl std::os::fd::AsFd) -> std::io::Result<()> {
    use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
    fcntl_setfl(pipe, fcntl_getfl(pipe)? | OFlags::NONBLOCK)?;
    Ok(())
}

/// Spawns a real child process on `PATH`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    /// Drain both nonblocking pipes while checking the child and one deadline.
    /// Completion follows the command's exit, not inherited pipe EOF: SSH
    /// control processes may retain a descriptor after Herdr has finished.
    fn run(&self, program: &str, args: &[&str], timeout: Duration) -> Result<Output, ProcessError> {
        let spawn_error = |reason: String| ProcessError::Spawn {
            program: program.to_owned(),
            reason,
        };
        // Its own process group, so a timeout can kill whatever the child started
        // (a wrapper script's `sleep`) and not only the child.
        let mut child = Command::new(program)
            .args(args)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| spawn_error(e.to_string()))?;
        let deadline = Instant::now() + timeout;
        let timed_out = || ProcessError::Timeout {
            program: program.to_owned(),
            secs: timeout.as_secs(),
        };

        let Some(mut stdout_pipe) = child.stdout.take() else {
            kill_group(&mut child);
            return Err(spawn_error("stdout pipe unavailable".into()));
        };
        let Some(mut stderr_pipe) = child.stderr.take() else {
            kill_group(&mut child);
            return Err(spawn_error("stderr pipe unavailable".into()));
        };
        if let Err(error) = nonblocking(&stdout_pipe).and_then(|()| nonblocking(&stderr_pipe)) {
            kill_group(&mut child);
            return Err(spawn_error(error.to_string()));
        }
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        loop {
            if let Err(error) = capture_available(&mut stdout_pipe, &mut stdout)
                .and_then(|()| capture_available(&mut stderr_pipe, &mut stderr))
            {
                kill_group(&mut child);
                return Err(spawn_error(error.to_string()));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    // The child may have written between the preceding read
                    // and its exit. Read the buffered tail, without waiting for
                    // a detached descendant's inherited pipe to close.
                    let _ = capture_available(&mut stdout_pipe, &mut stdout);
                    let _ = capture_available(&mut stderr_pipe, &mut stderr);
                    return Ok(Output {
                        status_code: status.code().unwrap_or(-1),
                        success: status.success(),
                        stdout,
                        stderr,
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    kill_group(&mut child);
                    return Err(spawn_error(error.to_string()));
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                kill_group(&mut child);
                return Err(timed_out());
            }
            thread::sleep(remaining.min(Duration::from_millis(5)));
        }
    }
}

/// Kills `child` and its whole process group, then reaps it. The group is killed
/// through `kill(1)` (`kill -KILL -- -<pgid>`) because signalling a group needs
/// `unsafe` in Rust and this workspace forbids it; without a `kill` binary only
/// the child is killed, as before.
fn kill_group(child: &mut std::process::Child) {
    let group = format!("-{}", child.id());
    for kill in ["/bin/kill", "/usr/bin/kill", "kill"] {
        let ran = Command::new(kill)
            .args(["-KILL", "--", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if ran.is_ok_and(|s| s.success()) {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Selected fields of herdr's externally owned envelope. Metadata such as
/// request IDs is ignored. Keep the result as raw JSON so decoding T retains
/// field/type diagnostics and rejects duplicate fields in the command payload.
#[derive(Deserialize)]
struct Envelope {
    #[serde(default, deserialize_with = "present_result")]
    result: Option<Box<serde_json::value::RawValue>>,
    error: Option<ErrorBody>,
}

// An explicitly present null is a legitimate payload for T = Option<_> or ().
// The outer Option tracks absence, not the payload's nullability.
fn present_result<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Box<serde_json::value::RawValue>>, D::Error> {
    Box::<serde_json::value::RawValue>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

/// Decodes herdr's documented JSON envelope as `T` from whichever stream
/// carries it (stdout on success, stderr on failure) — a pure function, so
/// it's testable against captured real output with no process involved.
///
/// # Errors
///
/// [`ProcessError::Reported`] when the envelope itself reports a failure;
/// [`ProcessError::BadOutput`] when a successful exit's stdout wasn't the
/// envelope (or not the shape `T` expects); [`ProcessError::UnexpectedExit`]
/// when a nonzero exit's stderr wasn't the envelope either.
pub fn decode<T: serde::de::DeserializeOwned>(
    program: &str,
    stdout: &[u8],
    stderr: &[u8],
    success: bool,
) -> Result<T, ProcessError> {
    let bytes = if success { stdout } else { stderr };
    let bad_output = |reason: String| {
        if success {
            ProcessError::BadOutput {
                program: program.to_owned(),
                reason,
            }
        } else {
            ProcessError::UnexpectedExit {
                program: program.to_owned(),
                status: -1,
                stderr: String::from_utf8_lossy(stderr).into_owned(),
            }
        }
    };
    // Select the envelope before decoding the command's payload. If T is inside
    // an untagged enum, serde discards its actual error (e.g. a missing field)
    // and reports only that no envelope variant matched.
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|error| bad_output(error.to_string()))?;
    if let Some(error) = envelope.error {
        return Err(ProcessError::Reported {
            code: error.code,
            message: error.message,
        });
    }
    let result = envelope
        .result
        .ok_or_else(|| bad_output("missing result or error".into()))?;
    serde_json::from_str(result.get()).map_err(|error| bad_output(error.to_string()))
}

/// Runs `program <args>` through `runner` and decodes its documented JSON
/// envelope as `T`.
///
/// # Errors
///
/// See [`ProcessRunner::run`] and [`decode`].
pub fn call<T: serde::de::DeserializeOwned>(
    runner: &impl ProcessRunner,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<T, ProcessError> {
    let out = runner.run(program, args, timeout)?;
    match decode(program, &out.stdout, &out.stderr, out.success) {
        // `decode`'s UnexpectedExit doesn't know the real exit status (it
        // only sees the bytes); fill in the one `run` actually observed.
        Err(ProcessError::UnexpectedExit {
            program, stderr, ..
        }) => Err(ProcessError::UnexpectedExit {
            program,
            status: out.status_code,
            stderr,
        }),
        other => other,
    }
}

/// Prefixes `args` with `--machine <machine>` when given, so every call site
/// that supports targeting a saved herdr machine builds argv the same way.
#[must_use]
pub fn with_machine<'a>(machine: Option<&'a str>, args: &[&'a str]) -> Vec<&'a str> {
    let mut full = Vec::with_capacity(args.len() + 2);
    if let Some(m) = machine {
        full.push("--machine");
        full.push(m);
    }
    full.extend_from_slice(args);
    full
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str, timeout: Duration) -> Result<Output, ProcessError> {
        RealProcessRunner.run("sh", &["-c", script], timeout)
    }

    #[test]
    fn completed_child_does_not_wait_for_a_descendants_inherited_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("descendant.pid");
        let script = format!(
            "sleep 30 & echo $! > '{}'; printf 'complete'; printf 'note' >&2; exit 7",
            pidfile.display()
        );
        let result = sh(&script, Duration::from_secs(2));
        // Clean up only the descendant this test created, even on failure.
        if let Ok(pid) = std::fs::read_to_string(&pidfile) {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", pid.trim()])
                .status();
        }
        let output = result.unwrap();
        assert_eq!(output.status_code, 7);
        assert!(!output.success);
        assert_eq!(output.stdout, b"complete");
        assert_eq!(output.stderr, b"note");
    }

    #[test]
    fn output_capture_is_capped_while_excess_bytes_are_drained() {
        let output = sh(
            "head -c 9000000 /dev/zero; head -c 9000000 /dev/zero >&2",
            Duration::from_secs(60),
        )
        .unwrap();
        assert!(output.success);
        assert_eq!(output.stdout.len(), MAX_CAPTURE_BYTES as usize);
        assert_eq!(output.stderr.len(), MAX_CAPTURE_BYTES as usize);
    }

    #[test]
    fn closed_output_pipes_do_not_hide_a_still_running_child() {
        let error = sh(
            "exec >/dev/null 2>&1; exec sleep 30",
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(matches!(error, ProcessError::Timeout { .. }));
    }

    /// AGE-2026: output past the OS pipe buffer (64 KiB on Linux) used to
    /// block the child until the timeout, because it was read only after exit.
    #[test]
    fn output_larger_than_the_pipe_buffer_is_captured_without_stalling() {
        let out = sh(
            "head -c 2000000 /dev/zero; head -c 1500000 /dev/zero >&2",
            Duration::from_secs(60),
        )
        .unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.len(), 2_000_000);
        assert_eq!(out.stderr.len(), 1_500_000);
    }

    #[test]
    fn a_chatty_hung_child_is_still_killed_at_the_deadline() {
        // Fills the pipes and then hangs: the drains keep it unblocked, so
        // only the deadline can end it.
        let err = sh(
            "head -c 300000 /dev/zero; exec sleep 300",
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(matches!(err, ProcessError::Timeout { .. }), "{err:?}");
    }

    #[test]
    fn exit_status_and_both_streams_come_back() {
        let out = sh("echo out; echo err >&2; exit 3", Duration::from_secs(60)).unwrap();
        assert_eq!((out.success, out.status_code), (false, 3));
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
    }

    /// Output past the cap is discarded, not buffered without bound, and the
    /// child still runs to completion.
    #[test]
    fn output_past_the_capture_cap_is_truncated() {
        let out = sh("head -c 9000000 /dev/zero", Duration::from_secs(60)).unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.len() as u64, MAX_CAPTURE_BYTES);
    }

    #[test]
    fn decodes_a_real_ok_envelope() {
        #[derive(Deserialize, Debug, PartialEq, Eq)]
        struct R {
            x: u32,
        }
        let r: R = decode("herdr", br#"{"result":{"x":7}}"#, b"", true).unwrap();
        assert_eq!(r, R { x: 7 });
    }

    /// Trace: FR-048-AC-8
    #[test]
    fn payload_failure_names_the_missing_field_or_invalid_type() {
        #[derive(Debug, Deserialize)]
        struct Payload {
            status: String,
        }
        for (bytes, expected) in [
            (
                br#"{"id":"request","result":{}}"#.as_slice(),
                "missing field `status`",
            ),
            (
                br#"{"result":{"status":42}}"#.as_slice(),
                "expected a string",
            ),
        ] {
            let error = decode::<Payload>("herdr", bytes, b"", true).unwrap_err();
            let ProcessError::BadOutput { program, reason } = error else {
                panic!("expected a payload decode error");
            };
            assert_eq!(program, "herdr");
            assert!(reason.contains(expected), "{reason}");
            assert!(!reason.contains("untagged"));
        }
        let value =
            decode::<Payload>("herdr", br#"{"result":{"status":"working"}}"#, b"", true).unwrap();
        assert_eq!(value.status, "working");
    }

    /// Trace: FR-048-AC-8
    #[test]
    fn raw_payload_preserves_duplicate_rejection_and_null_distinction() {
        #[derive(Debug, Deserialize)]
        struct Payload {
            status: String,
        }
        let error = decode::<Payload>(
            "herdr",
            br#"{"result":{"status":"idle","status":"working"}}"#,
            b"",
            true,
        )
        .unwrap_err();
        assert!(
            matches!(error, ProcessError::BadOutput { reason, .. } if reason.contains("duplicate field `status`"))
        );
        let value = decode::<Payload>(
            "herdr",
            br#"{"id":"request","result":{"status":"working"}}"#,
            b"",
            true,
        )
        .unwrap();
        assert_eq!(value.status, "working");
        assert_eq!(
            decode::<Option<Payload>>("herdr", br#"{"result":null}"#, b"", true)
                .unwrap()
                .map(|v| v.status),
            None
        );
        let error =
            decode::<Option<Payload>>("herdr", br#"{"id":"request"}"#, b"", true).unwrap_err();
        assert!(
            matches!(error, ProcessError::BadOutput { reason, .. } if reason == "missing result or error")
        );
    }

    /// Trace: FR-048-AC-8
    #[test]
    fn payload_decode_failure_on_nonzero_exit_keeps_exit_semantics() {
        #[derive(Debug, Deserialize)]
        struct Payload {
            _status: String,
        }
        let stderr = br#"{"result":{}}"#;
        assert_eq!(
            decode::<Payload>("herdr", b"", stderr, false).unwrap_err(),
            ProcessError::UnexpectedExit {
                program: "herdr".into(),
                status: -1,
                stderr: String::from_utf8_lossy(stderr).into_owned(),
            }
        );
        // An error envelope does not need to contain any command payload fields.
        let error = decode::<Payload>(
            "herdr",
            br#"{"error":{"code":"busy","message":"later"}}"#,
            b"",
            true,
        )
        .unwrap_err();
        assert_eq!(
            error,
            ProcessError::Reported {
                code: "busy".into(),
                message: "later".into()
            }
        );
    }

    #[test]
    fn decodes_a_real_error_envelope_from_stderr() {
        let stderr =
            br#"{"error":{"code":"agent_not_found","message":"agent target w999:p999 not found"}}"#;
        let err = decode::<serde_json::Value>("herdr", b"", stderr, false).unwrap_err();
        assert!(
            matches!(&err, ProcessError::Reported { code, .. } if code == "agent_not_found"),
            "{err}"
        );
    }

    #[test]
    fn bad_json_on_success_is_bad_output_not_unexpected_exit() {
        let err = decode::<serde_json::Value>("herdr", b"not json", b"", true).unwrap_err();
        assert!(matches!(&err, ProcessError::BadOutput { program, .. } if program == "herdr"));
    }

    #[test]
    fn non_envelope_on_failure_is_unexpected_exit() {
        let err =
            decode::<serde_json::Value>("herdr", b"", b"usage: herdr ...", false).unwrap_err();
        assert!(matches!(&err, ProcessError::UnexpectedExit { program, .. } if program == "herdr"));
    }

    #[test]
    fn definitely_not_sent_is_true_only_for_documented_pre_input_rejections() {
        let not_sent = [
            ProcessError::Spawn {
                program: "herdr".into(),
                reason: "no such file".into(),
            },
            ProcessError::Reported {
                code: "agent_blocked".into(),
                message: "blocked".into(),
            },
            ProcessError::Reported {
                code: "agent_not_found".into(),
                message: "gone".into(),
            },
        ];
        for e in not_sent {
            assert!(
                e.definitely_not_sent(),
                "{e:?} should be definitely-not-sent"
            );
        }

        let ambiguous = [
            ProcessError::Reported {
                code: "agent_prompt_stalled".into(),
                message: "stalled".into(),
            },
            ProcessError::Reported {
                code: "some_future_code".into(),
                message: "?".into(),
            },
            ProcessError::UnexpectedExit {
                program: "herdr".into(),
                status: 137,
                stderr: String::new(),
            },
            ProcessError::BadOutput {
                program: "herdr".into(),
                reason: "garbage".into(),
            },
            ProcessError::Timeout {
                program: "herdr".into(),
                secs: 10,
            },
        ];
        for e in ambiguous {
            assert!(!e.definitely_not_sent(), "{e:?} should be ambiguous");
        }
    }

    #[test]
    fn with_machine_prefixes_only_when_given() {
        assert_eq!(
            with_machine(Some("hub"), &["agent", "list"]),
            vec!["--machine", "hub", "agent", "list"]
        );
        assert_eq!(
            with_machine(None, &["agent", "list"]),
            vec!["agent", "list"]
        );
    }

    struct RecordingRunner {
        calls: std::cell::RefCell<Vec<(String, Vec<String>, Duration)>>,
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
            Ok(self.response.clone())
        }
    }

    #[test]
    fn call_runs_the_program_with_exactly_the_given_argv_and_timeout() {
        let runner = RecordingRunner {
            calls: std::cell::RefCell::default(),
            response: Output {
                status_code: 0,
                success: true,
                stdout: br#"{"result":{"ok":true}}"#.to_vec(),
                stderr: Vec::new(),
            },
        };
        let _: serde_json::Value =
            call(&runner, "herdr", &["agent", "list"], Duration::from_secs(5)).unwrap();
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "herdr");
        assert_eq!(calls[0].1, vec!["agent", "list"]);
        assert_eq!(calls[0].2, Duration::from_secs(5));
    }

    /// `RealProcessRunner` actually enforces its timeout: a genuinely
    /// long-running child gets killed rather than left to block forever.
    /// Uses `sleep` (not `herdr`) as the spawned program — `ProcessRunner` is
    /// generic over the program name, so this exercises the real spawn/poll/
    /// kill path without depending on herdr being installed.
    #[test]
    fn real_process_runner_kills_a_hung_child_at_its_timeout() {
        let result = RealProcessRunner.run("sleep", &["5"], Duration::from_millis(300));
        assert!(
            matches!(&result, Err(ProcessError::Timeout { .. })),
            "{result:?}"
        );
    }

    /// A wrapper script that does not `exec` its child leaves a grandchild behind
    /// if only the child is killed. The whole process group is killed: the
    /// grandchild (its pid is written to a file by the script) is gone soon after
    /// the timeout.
    #[test]
    fn a_timeout_kills_what_a_wrapper_script_started_too() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let script = dir.path().join("wrapper.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n/bin/sleep 31 &\necho $! > {}\nwait\n",
                pidfile.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let result =
            RealProcessRunner.run(script.to_str().unwrap(), &[], Duration::from_millis(600));
        assert!(
            matches!(&result, Err(ProcessError::Timeout { .. })),
            "{result:?}"
        );
        let pid = std::fs::read_to_string(&pidfile).unwrap().trim().to_owned();
        // A zombie is dead: it only waits to be reaped.
        let alive = || {
            std::fs::read_to_string(std::path::Path::new("/proc").join(&pid).join("stat"))
                .is_ok_and(|stat| {
                    stat.rsplit(')')
                        .next()
                        .is_some_and(|rest| !rest.trim_start().starts_with('Z'))
                })
        };
        if !std::path::Path::new("/proc/self").exists() {
            return; // no /proc to look in (macOS): the kill itself is what ran
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive() {
            assert!(Instant::now() < deadline, "grandchild {pid} still running");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn real_process_runner_reports_spawn_failure_for_a_missing_binary() {
        let result = RealProcessRunner.run(
            "ix-relay-definitely-not-a-real-binary",
            &[],
            Duration::from_secs(1),
        );
        assert!(
            matches!(result, Err(ProcessError::Spawn { .. })),
            "{result:?}"
        );
    }
}
