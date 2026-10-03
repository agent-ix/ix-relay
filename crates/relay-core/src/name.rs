//! Agent names and message targets.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Maximum encoded storage identity size.
pub(crate) const MAX_NAME_LEN: usize = 2048;

/// Reserved for broadcast; no agent may take it.
pub(crate) const ALL: &str = "all";

/// A storage identity or signed message actor.
///
/// Leads use [`AgentName::for_session`] for durable queues. Public sender actors
/// use the verified three-part location. This type is not a routing resolver;
/// the CLI validates public locations before selecting any queue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentName(String);

impl AgentName {
    /// The name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AgentName {
    /// Whether `s` satisfies the name rule.
    fn is_valid(s: &str) -> bool {
        if let Some(rest) = s.strip_prefix("session/") {
            return !rest.is_empty()
                && s.len() <= MAX_NAME_LEN
                && !s.chars().any(|c| c.is_control() || c.is_whitespace());
        }
        if s.split(':').count() == 3 && s.parse::<crate::Address>().is_ok() {
            return s.len() <= crate::MAX_LABEL_BYTES;
        }
        let mut chars = s.chars();
        s.len() <= 32
            && s != ALL
            && chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    }

    /// Opaque durable queue identity, independent of mutable location labels.
    pub fn for_session(machine: &crate::MachineName, session: &crate::SessionId) -> Self {
        let encode = |s: &str| {
            s.as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        Self(format!(
            "session/{}/{}",
            encode(machine.as_str()),
            encode(session.as_str())
        ))
    }
}

impl FromStr for AgentName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        if Self::is_valid(s) {
            Ok(Self(s.to_owned()))
        } else {
            Err(Error::InvalidName { name: s.to_owned() })
        }
    }
}

impl TryFrom<String> for AgentName {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        s.parse()
    }
}

impl From<AgentName> for String {
    fn from(name: AgentName) -> Self {
        name.0
    }
}

impl fmt::Display for AgentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who a message is addressed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Target {
    /// One lead.
    Agent(AgentName),
    /// Every registered lead except the sender.
    All,
}

impl FromStr for Target {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        if s == ALL {
            Ok(Self::All)
        } else {
            s.parse().map(Self::Agent)
        }
    }
}

impl TryFrom<String> for Target {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        s.parse()
    }
}

impl From<Target> for String {
    fn from(target: Target) -> Self {
        match target {
            Target::Agent(name) => name.into(),
            Target::All => ALL.to_owned(),
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Agent(name) => name.fmt(f),
            Self::All => f.write_str(ALL),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_herdr_style_names() {
        for ok in ["a", "qsl-lead", "tl_lead2", &format!("a{}", "b".repeat(31))] {
            assert_eq!(ok.parse::<AgentName>().unwrap().as_str(), ok);
        }
    }

    #[test]
    fn rejects_names_herdr_would_reject() {
        let too_long = format!("a{}", "b".repeat(32));
        for bad in [
            "", "1lead", "-lead", "Lead", "qsl lead", "qsl.lead", "all", &too_long,
        ] {
            assert!(
                matches!(bad.parse::<AgentName>(), Err(Error::InvalidName { name }) if name == bad),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn target_all_is_broadcast_and_names_are_agents() {
        assert_eq!("all".parse::<Target>().unwrap(), Target::All);
        assert_eq!(
            "qsl-lead".parse::<Target>().unwrap(),
            Target::Agent("qsl-lead".parse().unwrap())
        );
        assert!("Qsl".parse::<Target>().is_err());
    }

    #[test]
    fn names_round_trip_through_json_and_bad_names_do_not_decode() {
        let target: Target = serde_json::from_str("\"all\"").unwrap();
        assert_eq!(serde_json::to_string(&target).unwrap(), "\"all\"");
        assert!(serde_json::from_str::<AgentName>("\"Bad Name\"").is_err());
    }
}

#[cfg(test)]
mod queue_identity_properties {
    #![allow(clippy::unwrap_used, reason = "property test fixtures")]
    use super::*;
    use proptest::prelude::*;
    proptest! {
        #[test]
        fn encoded_queue_identity_round_trips_full_session_strings(
            machine in "[a-z]{1,64}", session in "[a-zA-Z0-9 :/_-]{1,256}"
        ) {
            let machine: crate::MachineName = machine.parse().unwrap();
            let session: crate::SessionId = session.parse().unwrap();
            let id = AgentName::for_session(&machine, &session);
            prop_assert_eq!(id.as_str().parse::<AgentName>().unwrap(), id.clone());
            prop_assert_ne!(id.clone(), AgentName::for_session(&"another-server".parse().unwrap(), &session));
            let other: crate::SessionId = if session.as_str() == "different-session" { "other-session" } else { "different-session" }.parse().unwrap();
            prop_assert_ne!(id, AgentName::for_session(&machine, &other));
        }
    }
}
