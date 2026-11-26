//! Location identities backed by genuine harness session associations.
use crate::{budget::Budget, hook::Event};
use ix_trace_rs::implements;
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
#[implements("FR-074-AC-1")]
fn caller_claude_record() -> Option<claude_wire::SessionRecord> {
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
    let directory = std::env::var_os("IX_RELAY_CLAUDE_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".claude"))
        })?
        .join("sessions");
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
                    return Some(record);
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

fn caller_claude_session() -> Option<SessionId> {
    caller_claude_record()?.session_id.parse().ok()
}

/// Verify the explicit placement recovery from the caller's own live socket
/// record and the operator-named Herdr pane.
#[implements("FR-074-AC-1")]
pub fn claim_socket_placement(
    relay: &mut Relay,
    machine: &MachineName,
    address: &relay_core::Address,
    pane: &relay_core::PaneId,
) -> Result<relay_core::Registration, Box<dyn std::error::Error>> {
    let record = caller_claude_record().ok_or("caller has no verified Claude process record")?;
    #[cfg(unix)]
    let socket_ok = caller_socket_is_private(&record);
    #[cfg(not(unix))]
    let socket_ok = false;
    if !matches!(record.verdict(), claude_wire::RecordVerdict::Supported) || !socket_ok {
        return Err("caller session does not have its own verified live inbox socket".into());
    }
    let session: SessionId = record.session_id.parse()?;
    enrollment::claim_socket_placement(
        relay,
        machine,
        &session,
        address,
        pane,
        SESSION_START_HERDR_BUDGET,
    )
}

#[cfg(unix)]
#[implements("FR-074-AC-1")]
fn caller_socket_is_private(record: &claude_wire::SessionRecord) -> bool {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt},
    };
    let Some(path) = record.messaging_socket_path.as_ref() else {
        return false;
    };
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return false;
    };
    let Ok(home_meta) = std::fs::metadata(home) else {
        return false;
    };
    let uid = home_meta.uid();
    let Some(file_name) = path.file_name() else {
        return false;
    };
    if file_name.as_bytes() != format!("{}.sock", record.pid).as_bytes() {
        return false;
    }
    let mut roots = Vec::new();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        roots.push(std::path::PathBuf::from(runtime).join("cc-socks"));
    }
    #[cfg(target_os = "macos")]
    roots.push(std::path::PathBuf::from("/tmp/cc-socks"));
    #[cfg(target_os = "linux")]
    roots.push(std::path::PathBuf::from(format!("/tmp/cc-socks-{uid}")));
    let Some(root) = roots
        .iter()
        .find(|root| path.parent() == Some(root.as_path()))
    else {
        return false;
    };
    let Ok(root_meta) = std::fs::symlink_metadata(root) else {
        return false;
    };
    if root_meta.file_type().is_symlink()
        || !root_meta.is_dir()
        || root_meta.mode() & 0o077 != 0
        || root_meta.uid() != uid
    {
        return false;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    meta.file_type().is_socket() && meta.uid() == uid && meta.mode() & 0o077 == 0
}

/// Interactive commands heal their own enrollment from live session evidence.
pub fn derive(relay: &mut Relay, machine: &MachineName) -> Result<Option<Identity>, IdentityError> {
    let session = std::env::var("CODEX_THREAD_ID")
        .ok()
        .and_then(|value| value.parse::<SessionId>().ok())
        .or_else(caller_claude_session);
    if let Some(session) = session
        && let Some(registration) = relay.directory_session(machine, &session)?
    {
        // Refresh metadata from the same fresh snapshot before shortcut expansion.
        enrollment::enroll(relay, &registration)?;
        return Ok(Some(Identity {
            name: registration.name,
            source: Source::Location,
        }));
    }
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
