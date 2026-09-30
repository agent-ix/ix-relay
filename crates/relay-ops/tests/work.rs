//! Measured monitor binding through the external command seam.
use herdr_cli::{Output, ProcessError, ProcessRunner};
use relay_ops::{
    OpsError,
    work::{observe_work_binding, resolve_work_target},
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
                format!(",{}", pane.replace("w1:p1", "w1:p2"))
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
