//! Location identities backed by genuine harness session associations.
use crate::{budget::Budget, hook::Event};
use relay_core::{AgentName, IdentityCommit, MachineName, Relay, SessionId, SessionProbe};
use relay_ops::enrollment;
use std::time::Duration;

pub(crate) const SESSION_START_HERDR_BUDGET: Duration = Duration::from_secs(3);
pub(crate) const LAZY_HERDR_BUDGET: Duration = Duration::from_millis(600);

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error(transparent)]
    Relay(#[from] relay_core::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Location,
}
impl Source {
    pub fn as_str(self) -> &'static str {
        "location"
    }
}
#[derive(Debug, Clone)]
pub struct Identity {
    pub name: AgentName,
    pub source: Source,
}
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum WhoamiJson<'a> {
    Lead {
        name: &'a str,
        source: &'a str,
        address: Option<&'a str>,
    },
    NotALead {
        lead: bool,
    },
}
impl<'a> WhoamiJson<'a> {
    pub fn lead(_identity: &'a Identity, address: Option<&'a str>) -> Self {
        Self::Lead {
            name: address.unwrap_or("unverified"),
            source: "location",
            address,
        }
    }
    pub fn not_a_lead() -> Self {
        Self::NotALead { lead: false }
    }
}

#[derive(Debug, Clone)]
pub struct Derived {
    pub name: AgentName,
    pub commit: IdentityCommit,
}
impl Derived {
    pub fn cached(name: AgentName) -> Self {
        Self {
            name,
            commit: IdentityCommit::default(),
        }
    }
}
#[derive(Debug, Clone)]
pub enum HookIdentity {
    NotALead,
    Suppress { session_id: Option<SessionId> },
    Lead(Box<Derived>),
    HerdrFailed { budget_spent: bool },
}
pub struct HookRequest<'a> {
    pub session_id: Option<&'a SessionId>,
    pub source: Option<&'a str>,
    pub event: Event,
}

fn process_registration(
    machine: &MachineName,
) -> Result<relay_core::Registration, enrollment::EnrollmentError> {
    let session = if let Ok(session) = std::env::var("CODEX_THREAD_ID") {
        session.parse()?
    } else {
        caller_claude_session().ok_or("caller has no genuine harness session identity")?
    };
    enrollment::current(machine, &session, SESSION_START_HERDR_BUDGET)
}

/// A Claude CLI inherits its harness as an ancestor. Match that process's own
/// record and start time; inherited pane variables never establish ownership.
fn caller_claude_session() -> Option<SessionId> {
    use herdr_cli::ProcessRunner;
    let output = herdr_cli::RealProcessRunner
        .run(
            "/usr/bin/env",
            &["LC_ALL=C", "TZ=UTC", "ps", "-axo", "pid=,ppid=,lstart="],
            Duration::from_secs(1),
        )
        .ok()?;
    if !output.success {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let processes: std::collections::HashMap<u32, (u32, String)> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            Some((pid, (parent, fields.collect::<Vec<_>>().join(" "))))
        })
        .collect();
    let home = std::env::var_os("HOME")?;
    let directory = std::path::PathBuf::from(home).join(".claude/sessions");
    let mut pid = std::process::id();
    for _ in 0..32 {
        let (parent, _mac_start) = processes.get(&pid)?;
        if let Ok(file) = std::fs::File::open(directory.join(format!("{pid}.json"))) {
            use std::io::Read;
            if let Ok(record) =
                serde_json::from_reader::<_, claude_wire::SessionRecord>(file.take(64 * 1024))
            {
                #[cfg(target_os = "macos")]
                let same_start = record.proc_start.as_deref().is_some_and(|s| {
                    s.split_whitespace().collect::<Vec<_>>().join(" ") == *_mac_start
                });
                #[cfg(not(target_os = "macos"))]
                let same_start =
                    record.proc_start == relay_core::socket_status::proc_start_time(pid);
                if record.pid == pid && same_start {
                    return record.session_id.parse().ok();
                }
            }
        }
        if *parent == 0 || *parent == pid {
            break;
        }
        pid = *parent;
    }
    None
}

/// Interactive commands heal their own enrollment from live session evidence.
pub fn derive(relay: &mut Relay, machine: &MachineName) -> Result<Option<Identity>, IdentityError> {
    match process_registration(machine) {
        Ok(registration) => {
            enrollment::enroll(relay, &registration)?;
            Ok(Some(Identity {
                name: registration.name,
                source: Source::Location,
            }))
        }
        Err(error) => {
            crate::report::line(format_args!(
                "agent-msg: {error}; manual registration cannot repair stale caller context"
            ));
            Ok(None)
        }
    }
}
pub fn derive_for_hook(
    _relay: &mut Relay,
    machine: &MachineName,
    request: &HookRequest<'_>,
    probe: &SessionProbe,
    budget: &Budget<'_>,
) -> Result<HookIdentity, IdentityError> {
    let Some(session) = request.session_id else {
        return Ok(HookIdentity::NotALead);
    };
    if request.source == Some("fork") || probe.suppressed {
        return Ok(HookIdentity::Suppress {
            session_id: Some(session.clone()),
        });
    }
    if request.event != Event::SessionStart
        && let Some(name) = &probe.name
        && *name == AgentName::for_session(machine, session)
    {
        return Ok(HookIdentity::Lead(Box::new(Derived::cached(name.clone()))));
    }
    let Some(left) = budget.remaining() else {
        return Ok(HookIdentity::HerdrFailed { budget_spent: true });
    };
    match enrollment::current(machine, session, left.min(SESSION_START_HERDR_BUDGET)) {
        Ok(registration) => {
            let name = registration.name.clone();
            Ok(HookIdentity::Lead(Box::new(Derived {
                name: name.clone(),
                commit: IdentityCommit {
                    registration: Some(registration),
                    session_name: Some((session.clone(), name)),
                    ..IdentityCommit::default()
                },
            })))
        }
        Err(error) => {
            crate::report::line(format_args!("agent-msg startup: {error}"));
            Ok(HookIdentity::HerdrFailed {
                budget_spent: budget.remaining().is_none(),
            })
        }
    }
}
