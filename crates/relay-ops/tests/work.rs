// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Measured monitor binding through the external command seam.
use herdr_cli::{Output, ProcessError, ProcessRunner};
use ix_trace_rs::trace;
use relay_ops::{
    OpsError,
    work::{observe_work_binding, resolve_work_target, resolve_work_target_for_session},
};
use std::time::Duration;

struct Runner {
    session: Option<&'static str>,
    resume: &'static str,
    pid: u32,
    duplicate: bool,
    profile: bool,
    harness: &'static str,
}
impl ProcessRunner for Runner {
    fn run(&self, program: &str, args: &[&str], _: Duration) -> Result<Output, ProcessError> {
        assert_eq!(program, "herdr");
        let body = if args == ["api", "snapshot"] {
            let session = self.session.map_or("null".into(), |s| {
                format!(
                    r#"{{"agent":"{}","kind":"id","value":"{s}"}}"#,
                    self.harness
                )
            });
            let pane = format!(
                r#"{{"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","agent":"codex","agent_status":"working","agent_session":{session}}}"#
            );
            let other = if self.duplicate {
                let other = pane.replace("w1:p1", "w1:p2");
                let other = match self.session {
                    Some(session) => other.replace(session, "session-b"),
                    None => other.replace(
                        r#""agent_session":null"#,
                        r#""agent_session":{"agent":"codex","kind":"id","value":"session-b"}"#,
                    ),
                };
                format!(",{other}")
            } else {
                String::new()
            };
            format!(
                r#"{{"snapshot":{{"panes":[{pane}{other}],"workspaces":[{{"workspace_id":"w1","label":"SWM"}}],"tabs":[{{"tab_id":"w1:t1","label":"code"}}]}}}}"#
            )
        } else {
            assert_eq!(args, ["pane", "process-info", "--pane", "w1:p1"]);
            let argv = if self.profile {
                r#"["codex","--profile","resume","exec","task"]"#.into()
            } else {
                format!(r#"["codex","resume","{}"]"#, self.resume)
            };
            format!(
                r#"{{"process_info":{{"foreground_processes":[{{"name":"codex","pid":{},"argv":{}}}]}}}}"#,
                self.pid, argv
            )
        };
        Ok(Output {
            status_code: 0,
            success: true,
            stdout: format!(r#"{{"result":{body}}}"#).into_bytes(),
            stderr: vec![],
        })
    }
}

/// Trace: FR-048-AC-1
#[test]
fn binding_uses_measured_resume_identity_and_refuses_ambiguity_conflict_and_replacement() {
    let machine = "luna".parse().unwrap();
    let runner = Runner {
        session: None,
        resume: "session-a",
        pid: 42,
        duplicate: false,
        profile: false,
        harness: "codex",
    };
    let got = resolve_work_target(&runner, Some("luna:SWM:code"), &machine).unwrap();
    assert_eq!(got.binding.session.as_str(), "session-a");
    assert_eq!(got.binding.process_id, 42);
    assert_eq!(got.binding.pane.as_str(), "w1:p1");
    assert_eq!(got.status, "working");
    assert!(matches!(
        resolve_work_target(
            &Runner {
                profile: true,
                ..runner
            },
            Some("luna:SWM:code"),
            &machine
        ),
        Err(OpsError::WorkSessionUnavailable)
    ));
    assert!(matches!(
        resolve_work_target(
            &Runner {
                session: Some("session-a"),
                harness: "claude",
                ..runner
            },
            Some("luna:SWM:code"),
            &machine
        ),
        Err(OpsError::WorkIdentityConflict)
    ));
    assert!(matches!(
        resolve_work_target(
            &Runner {
                duplicate: true,
                ..runner
            },
            Some("luna:SWM:code"),
            &machine
        ),
        Err(OpsError::WorkTargetCardinality { matches: 2 })
    ));
    let expected_session = "session-a".parse().unwrap();
    let selected = resolve_work_target_for_session(
        &Runner {
            session: Some("session-a"),
            duplicate: true,
            ..runner
        },
        Some("luna:SWM:code"),
        Some(&expected_session),
        &machine,
    )
    .unwrap();
    assert_eq!(selected.binding.pane.as_str(), "w1:p1");
    assert_eq!(selected.binding.session, expected_session);
    let missing_session = "session-missing".parse().unwrap();
    assert!(matches!(
        resolve_work_target_for_session(
            &Runner {
                session: Some("session-a"),
                duplicate: true,
                ..runner
            },
            Some("luna:SWM:code"),
            Some(&missing_session),
            &machine,
        ),
        Err(OpsError::WorkSessionReplaced)
    ));
    assert!(matches!(
        resolve_work_target(
            &Runner {
                session: Some("other"),
                ..runner
            },
            Some("luna:SWM:code"),
            &machine
        ),
        Err(OpsError::WorkIdentityConflict)
    ));
    assert!(matches!(
        observe_work_binding(&Runner { pid: 43, ..runner }, &got.binding, &machine),
        Err(OpsError::WorkSessionReplaced)
    ));
    assert!(matches!(
        observe_work_binding(
            &Runner {
                resume: "session-b",
                ..runner
            },
            &got.binding,
            &machine
        ),
        Err(OpsError::WorkSessionReplaced)
    ));
}

struct ProcessDetails<R> {
    inner: R,
    argv: Option<serde_json::Value>,
    omit_processes: bool,
}
impl<R: ProcessRunner> ProcessRunner for ProcessDetails<R> {
    #[expect(
        clippy::expect_used,
        reason = "test fixture assertions fail if the fixed JSON shape changes"
    )]
    fn run(&self, program: &str, args: &[&str], timeout: Duration) -> Result<Output, ProcessError> {
        let mut output = self.inner.run(program, args, timeout)?;
        if args.first() == Some(&"pane") {
            let mut response: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("fixture runner emits JSON");
            let info = response["result"]["process_info"]
                .as_object_mut()
                .expect("fixture includes process info");
            if self.omit_processes {
                info.remove("foreground_processes");
            } else {
                let process = info["foreground_processes"][0]
                    .as_object_mut()
                    .expect("fixture includes one process");
                process.remove("argv");
                if let Some(argv) = &self.argv {
                    process.insert("argv".into(), argv.clone());
                }
            }
            output.stdout = serde_json::to_vec(&response).expect("JSON fixture serializes");
        }
        Ok(output)
    }
}

/// Trace: FR-048-AC-1, FR-048-AC-10
#[test]
fn optional_process_details_preserve_native_binding_and_refuse_missing_evidence() {
    let machine = "luna".parse().unwrap();
    for argv in [None, Some(serde_json::Value::Null)] {
        let runner = ProcessDetails {
            inner: Runner {
                session: Some("session-a"),
                resume: "session-a",
                pid: 42,
                duplicate: false,
                profile: false,
                harness: "codex",
            },
            argv,
            omit_processes: false,
        };
        let observed = resolve_work_target(&runner, Some("luna:SWM:code"), &machine).unwrap();
        assert_eq!(observed.binding.session.as_str(), "session-a");
        assert_eq!(observed.binding.process_id, 42);
        let unreported = ProcessDetails {
            inner: Runner {
                session: None,
                ..runner.inner
            },
            ..runner
        };
        assert!(matches!(
            resolve_work_target(&unreported, Some("luna:SWM:code"), &machine),
            Err(OpsError::WorkSessionUnavailable)
        ));
        let empty = ProcessDetails {
            omit_processes: true,
            ..unreported
        };
        assert!(matches!(
            resolve_work_target(&empty, Some("luna:SWM:code"), &machine),
            Err(OpsError::WorkProcessAmbiguous)
        ));
    }
    let invalid = ProcessDetails {
        inner: Runner {
            session: Some("session-a"),
            resume: "session-a",
            pid: 42,
            duplicate: false,
            profile: false,
            harness: "codex",
        },
        argv: Some(serde_json::json!("invalid")),
        omit_processes: false,
    };
    assert!(matches!(
        resolve_work_target(&invalid, Some("luna:SWM:code"), &machine),
        Err(OpsError::Herdr(ProcessError::BadOutput { .. }))
    ));
}

struct ClaudeProcesses {
    processes: serde_json::Value,
    session: serde_json::Value,
}

impl ClaudeProcesses {
    fn new(processes: serde_json::Value) -> Self {
        Self {
            processes,
            session: serde_json::json!({"agent":"claude", "kind":"id", "value":"session-a"}),
        }
    }
}

impl ProcessRunner for ClaudeProcesses {
    fn run(&self, program: &str, args: &[&str], _: Duration) -> Result<Output, ProcessError> {
        assert_eq!(program, "herdr");
        let result = if args == ["api", "snapshot"] {
            serde_json::json!({"snapshot": {
                "panes": [{"pane_id":"w1:p1", "workspace_id":"w1", "tab_id":"w1:t1",
                    "agent":"claude", "agent_status":"working", "agent_session":self.session}],
                "workspaces":[{"workspace_id":"w1", "label":"SWM"}],
                "tabs":[{"tab_id":"w1:t1", "label":"plan"}]
            }})
        } else {
            assert_eq!(args, ["pane", "process-info", "--pane", "w1:p1"]);
            serde_json::json!({"process_info":{"foreground_processes":self.processes}})
        };
        Ok(Output {
            status_code: 0,
            success: true,
            stdout: serde_json::json!({"result":result})
                .to_string()
                .into_bytes(),
            stderr: vec![],
        })
    }
}

#[test]
#[trace("FR-048-AC-1", "FR-048-AC-10")]
fn claude_binding_matches_exact_executable_basename_with_optional_arguments() {
    let machine = "draco".parse().unwrap();
    for process in [
        serde_json::json!({"name":"2.1.329", "argv0":"claude", "pid":42, "argv":["claude"]}),
        serde_json::json!({"name":"2.1.329", "argv0":"/Users/test/.local/bin/claude", "pid":42}),
        serde_json::json!({"name":"2.1.329", "argv0":"claude", "pid":42, "argv":null}),
        serde_json::json!({"name":"claude", "pid":42}),
        serde_json::json!({"name":"/Users/test/.local/bin/claude", "pid":42, "argv0":null, "argv":null}),
    ] {
        let runner = ClaudeProcesses::new(serde_json::json!([
            {"name":"caffeinate", "argv0":"/usr/bin/caffeinate", "pid":43},
            process,
            {"name":"filament-mcp", "argv":["filament-mcp", "claude"], "pid":44}
        ]));
        let observed = resolve_work_target(&runner, Some("draco:SWM:plan"), &machine).unwrap();
        assert_eq!(observed.binding.session.as_str(), "session-a");
        assert_eq!(observed.binding.process_id, 42);
        assert_eq!(observed.binding.kind, "claude");
        assert_eq!(observed.binding.address, "draco:SWM:plan");
    }
}

#[test]
#[trace("FR-048-AC-1", "FR-048-AC-10")]
fn claude_binding_refuses_helpers_ambiguous_candidates_and_zero_pid() {
    let machine = "draco".parse().unwrap();
    let process = serde_json::json!({"name":"2.1.329", "argv0":"claude", "pid":42});
    for processes in [
        serde_json::json!([]),
        serde_json::json!([process, process]),
        serde_json::json!([process, {"name":"/bin/claude", "pid":43}]),
        serde_json::json!([{"name":"2.1.329", "argv0":"claude", "pid":0}]),
        serde_json::json!([{"name":"2.1.329", "pid":42, "argv":["claude"]}]),
        serde_json::json!([{"name":"2.1.329", "argv0":null, "pid":42, "argv":["claude"]}]),
        serde_json::json!([{"name":"helper", "argv0":"helper", "pid":42, "argv":["helper", "claude"]}]),
        serde_json::json!([{"name":"claude-helper", "argv0":"/claude/helper", "pid":42}]),
    ] {
        assert!(matches!(
            resolve_work_target(
                &ClaudeProcesses::new(processes),
                Some("draco:SWM:plan"),
                &machine
            ),
            Err(OpsError::WorkProcessAmbiguous)
        ));
    }
    let runner = ClaudeProcesses {
        session: serde_json::Value::Null,
        ..ClaudeProcesses::new(serde_json::json!([process]))
    };
    assert!(matches!(
        resolve_work_target(&runner, Some("draco:SWM:plan"), &machine),
        Err(OpsError::WorkSessionUnavailable)
    ));
    let runner = ClaudeProcesses {
        session: serde_json::json!({"agent":"codex", "kind":"id", "value":"session-a"}),
        ..runner
    };
    assert!(matches!(
        resolve_work_target(&runner, Some("draco:SWM:plan"), &machine),
        Err(OpsError::WorkIdentityConflict)
    ));
}

#[test]
#[trace("FR-048-AC-10")]
fn claude_binding_refuses_malformed_optional_argument_types() {
    let machine = "draco".parse().unwrap();
    for process in [
        serde_json::json!({"name":"claude", "pid":42, "argv0":42}),
        serde_json::json!({"name":"claude", "pid":42, "argv0":["claude"]}),
        serde_json::json!({"name":"claude", "pid":42, "argv":"claude"}),
        serde_json::json!({"name":"claude", "pid":42, "argv":[42]}),
    ] {
        assert!(matches!(
            resolve_work_target(
                &ClaudeProcesses::new(serde_json::json!([process])),
                Some("draco:SWM:plan"),
                &machine,
            ),
            Err(OpsError::Herdr(ProcessError::BadOutput { .. }))
        ));
    }
}
