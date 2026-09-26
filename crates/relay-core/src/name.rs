//! Agent names and message targets.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Longest name herdr accepts; the bus uses the same rule so one name works in both.
pub(crate) const MAX_NAME_LEN: usize = 32;

/// Reserved for broadcast; no agent may take it.
pub(crate) const ALL: &str = "all";

/// A lead's name on the bus, e.g. `qsl-lead`.
///
/// Follows herdr's rule `[a-z][a-z0-9_-]{0,31}`, so the same name can address the
/// agent through `herdr agent prompt`. Unlike herdr names it is global across machines.
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
        let mut chars = s.chars();
        s.len() <= MAX_NAME_LEN
            && s != ALL
            && chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    }

    /// Wraps a string the caller has just built to satisfy the name rule by
    /// construction ([`crate::naming::slugify`], [`crate::naming::with_suffix`]),
    /// so those never need a panicking `parse().expect(..)` on a hook path.
    /// Their proptests are what check "by construction"; debug builds assert it.
    pub(crate) fn from_constructed(s: String) -> Self {
        debug_assert!(
            Self::is_valid(&s),
            "constructed an invalid agent name {s:?}"
        );
        Self(s)
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
