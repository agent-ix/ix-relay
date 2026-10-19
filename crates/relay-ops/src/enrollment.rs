//! Session-bound enrollment from current Herdr topology. No names or aliases.
use crate::find::{self, MachineView, RealFindHerdr};
use relay_core::{Address, AgentName, MachineName, Registration, Relay, SessionId};
use std::time::Duration;

/// Failure to verify a live lead or enroll its session.
pub type EnrollmentError = Box<dyn std::error::Error>;

/// Builds registrations only where Herdr provides an exact session association.
pub fn registrations(view: &MachineView) -> Result<Vec<Registration>, EnrollmentError> {
    if view.dropped != 0 {
        return Err(
            "Herdr topology contains undecodable entries; cannot verify a unique lead".into(),
        );
    }
    let places = find::places(view);
    let mut registrations = Vec::new();
    for agent in &view.agents {
        let Some(session) = &agent.session_id else {
            continue;
        };
        let Some(place) = places.get(&agent.pane_id) else {
            continue;
        };
        if place.tab.starts_with("esc-") {
            continue;
        }
        let address = place.tab_address(&view.machine);
        if validate_address(&address.to_string()).is_err() {
            continue;
        }
        registrations.push(Registration {
            name: AgentName::for_session(&view.machine, session),
            machine: view.machine.clone(),
            kind: agent.kind.clone(),
            session_id: Some(session.clone()),
            cwd: None,
            herdr_pane: Some(agent.pane_id.clone()),
            place: Some(place.clone()),
            last_seen_ms: now_ms(),
        });
    }
    Ok(registrations)
}

/// Public lead addresses always contain exactly server, workspace, and tab.
pub fn validate_address(input: &str) -> Result<Address, EnrollmentError> {
    let parts: Vec<_> = input.split(':').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || p.chars().any(char::is_control))
        || input.len() > relay_core::MAX_LABEL_BYTES
    {
        return Err(format!("invalid lead address {input:?}; use server:workspace:tab").into());
    }
    Ok(input.parse()?)
}

/// Enrolls a verified genuine session and primes hooks in one transaction.
/// A discovery enrollment must be usable by the very next hook without another
/// slow topology lookup. No record without a verified session is accepted.
pub fn enroll(relay: &mut Relay, registration: &Registration) -> relay_core::Result<()> {
    let session =
        registration
            .session_id
            .clone()
            .ok_or_else(|| relay_core::Error::UnverifiedEnrollment {
                reason: "enrollment requires a genuine session".into(),
            })?;
    if registration.verified_place().is_none()
        || registration.name != AgentName::for_session(&registration.machine, &session)
    {
        return Err(relay_core::Error::UnverifiedEnrollment {
            reason: "enrollment requires a verified canonical session queue".into(),
        });
    }
    relay.commit_identity(&relay_core::IdentityCommit {
        registration: Some(registration.clone()),
        session_name: Some((session, registration.name.clone())),
        ..Default::default()
    })?;
    Ok(())
}

/// Local session identity is proved by the live session ID, never a pane claim.
pub fn current(
    machine: &MachineName,
    session: &SessionId,
    timeout: Duration,
) -> Result<Registration, EnrollmentError> {
    let view = find::gather_machine_view(&RealFindHerdr::new(), None, machine, timeout)?;
    if view
        .agents
        .iter()
        .filter(|agent| agent.session_id.as_ref() == Some(session))
        .count()
        > 1
    {
        return Err("this session appears in multiple Herdr panes".into());
    }
    if let Some(agent) = view
        .agents
        .iter()
        .find(|agent| agent.session_id.as_ref() == Some(session))
        && let Some(place) = find::places(&view).get(&agent.pane_id)
    {
        validate_address(&place.tab_address(machine).to_string())?;
    }
    let live = registrations(&view)?;
    let mut matches = live
        .iter()
        .filter(|r| r.session_id.as_ref() == Some(session));
    let found = matches.next().ok_or("this session has no verified Herdr placement; start the terminal lead with codex --no-daemon")?;
    if matches.next().is_some() {
        return Err("this session appears in multiple Herdr panes".into());
    }
    if live
        .iter()
        .filter(|r| r.full_address() == found.full_address())
        .count()
        != 1
    {
        return Err("multiple live leads share this tab; use separate tabs".into());
    }
    Ok(found.clone())
}

/// Publish a complete verified LOCAL observation for cold fallback recovery.
/// Normal sender lookups and remote resolution do not invoke this operation.
pub fn refresh_local_directory(
    relay: &mut Relay,
    machine: &MachineName,
) -> Result<(), EnrollmentError> {
    // This cold machine fallback is not needed for a fresh directory hit.
    let revision = relay.begin_directory_observation(machine)?;
    let view =
        find::gather_machine_view(&RealFindHerdr::new(), None, machine, Duration::from_secs(3))?;
    let live = registrations(&view)?;
    for registration in &live {
        if live
            .iter()
            .filter(|other| {
                other.name == registration.name
                    || other.full_address() == registration.full_address()
            })
            .count()
            == 1
        {
            enroll(relay, registration)?;
        }
    }
    relay.publish_directory(machine, revision, &live)?;
    Ok(())
}

/// Resolve through the relay directory. Local Herdr is a fallback only; remote
/// resolution requests the owning watcher through Redis, never outbound SSH.
pub fn recipient(
    relay: &mut Relay,
    local: &MachineName,
    address: &Address,
) -> Result<relay_core::DirectoryRecipient, EnrollmentError> {
    let machine: MachineName = address.server().parse()?;
    match relay.directory_recipient(address)? {
        relay_core::DirectoryRead::Unique(recipient) => return Ok(*recipient),
        relay_core::DirectoryRead::Ambiguous => {
            return Err(format!("multiple live leads at {address}; use separate tabs").into());
        }
        _ => {}
    }
    if machine == *local {
        refresh_local_directory(relay, local)?;
    } else {
        relay.request_directory_refresh(&machine)?;
    }
    let deadline =
        std::time::Instant::now() + Duration::from_secs(if machine == *local { 3 } else { 60 });
    loop {
        match relay.directory_recipient(address)? {
            relay_core::DirectoryRead::Unique(recipient) => return Ok(*recipient),
            relay_core::DirectoryRead::Ambiguous => return Err(format!("multiple live leads at {address}; use separate tabs").into()),
            relay_core::DirectoryRead::Absent if machine == *local => return Err(format!("no session-verified live lead at {address}").into()),
            _ if std::time::Instant::now() >= deadline => return Err(format!("no fresh verified relay directory entry for {address}; destination watcher on {machine} must refresh its local Herdr topology; nothing queued").into()),
            _ => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test fixtures")]
mod tests {
    use super::*;

    #[test]
    fn directory_preserves_harness_kind_and_requires_genuine_session_for_every_adapter() {
        for kind in ["claude", "codex", "opencode"] {
            let session: SessionId = format!("{kind}-genuine-session").parse().unwrap();
            let mut view = MachineView {
                machine: "luna".parse().unwrap(),
                dropped: 0,
                agents: vec![find::HerdrAgent {
                    kind: kind.into(),
                    pane_id: "w1:p1".parse().unwrap(),
                    tab_id: "w1:t1".parse().unwrap(),
                    workspace_id: "w1".parse().unwrap(),
                    status: "idle".into(),
                    session_name: Some("legacy-alias".into()),
                    session_id: Some(session.clone()),
                }],
                workspaces: vec![find::HerdrWorkspace {
                    workspace_id: "w1".parse().unwrap(),
                    label: "IR".into(),
                    tab_count: 1,
                }],
                tabs: vec![find::HerdrTab {
                    tab_id: "w1:t1".parse().unwrap(),
                    label: "plan".into(),
                }],
            };
            let entries = registrations(&view).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].kind, kind);
            assert_eq!(entries[0].session_id, Some(session));
            assert_eq!(
                entries[0].full_address().unwrap().to_string(),
                "luna:IR:plan"
            );
            view.agents[0].session_id = None;
            assert!(registrations(&view).unwrap().is_empty());
        }
    }
}
