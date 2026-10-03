//! Session-bound enrollment from current Herdr topology. No names or aliases.
use crate::find::{self, FindHerdr, MachineView, RealFindHerdr};
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
            kind: "herdr".into(),
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

/// Refreshes an address's live topology and enrolls the uniquely identified lead.
pub fn recipient(
    relay: &mut Relay,
    local: &MachineName,
    address: &Address,
) -> Result<Registration, EnrollmentError> {
    let herdr = RealFindHerdr::new();
    let machine: MachineName = address.server().parse()?;
    let selector = if machine == *local {
        None
    } else {
        let mut profiles = herdr
            .machine_list(Duration::from_secs(2))?
            .into_iter()
            .filter(|p| {
                p.enabled && p.label.parse::<MachineName>().ok().as_ref() == Some(&machine)
            });
        let selected = profiles
            .next()
            .ok_or_else(|| format!("no enabled Herdr machine profile for {machine}"))?;
        if profiles.next().is_some() {
            return Err(format!("multiple Herdr profiles for {machine}").into());
        }
        Some(selected.label)
    };
    // A saved remote profile may need its first SSH connection established.
    // This is a human send, not a latency-limited hook: retain a bounded cold
    // connection budget without requiring a prior find to warm the transport.
    let timeout = if selector.is_some() {
        Duration::from_secs(30)
    } else {
        Duration::from_secs(3)
    };
    let view = find::gather_machine_view(&herdr, selector.as_deref(), &machine, timeout)?;
    let live = registrations(&view)?;
    let candidates: Vec<_> = live
        .iter()
        .filter(|r| {
            r.verified_place()
                .is_some_and(|p| p.tab_address(&machine) == *address)
        })
        .cloned()
        .collect();
    let [recipient] = candidates.as_slice() else {
        return Err(if candidates.is_empty() {
            let available: Vec<_> = find::places(&view).values().map(|p| p.tab_address(&machine).to_string()).collect();
            if !view.workspaces.iter().any(|w| w.label.eq_ignore_ascii_case(address.workspace())) {
                format!("workspace {} does not exist on {machine}; live addresses: {}", address.workspace(), available.join(", "))
            } else if !view.agents.iter().any(|agent| {
                view.workspaces.iter().any(|w| w.workspace_id == agent.workspace_id && w.label.eq_ignore_ascii_case(address.workspace()))
                    && view.tabs.iter().any(|t| t.tab_id == agent.tab_id && address.tab().is_some_and(|tab| t.label.eq_ignore_ascii_case(tab)))
            }) {
                format!("tab {} does not exist in {machine}:{}; live addresses: {}", address.tab().unwrap_or(""), address.workspace(), available.join(", "))
            } else {
                format!("no session-verified live lead at {address}; Herdr must associate the real session before enrollment")
            }
        } else { format!("multiple live leads at {address}; use separate tabs") }.into());
    };
    if view
        .agents
        .iter()
        .filter(|agent| agent.session_id == recipient.session_id)
        .count()
        != 1
    {
        return Err(format!(
            "session at {address} appears in multiple live Herdr panes; refusing enrollment"
        )
        .into());
    }
    enroll(relay, recipient)?;
    Ok(recipient.clone())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
