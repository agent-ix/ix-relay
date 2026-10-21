//! Shared operator-set lane metadata used by `agent-msg who`.

use serde::{Deserialize, Serialize};

use crate::{AgentName, MachineName};

/// Maximum number of operator rule strings recorded for one server.
pub const MAX_OWNER_RULES: usize = 32;
/// Maximum UTF-8 bytes in one operator rule.
pub const MAX_OWNER_RULE_BYTES: usize = 500;

/// A server capacity value violates a storage or display bound.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServerCapacityError {
    /// CPU core count must be positive.
    #[error("cores must be greater than zero")]
    ZeroCores,
    /// RAM amount must be positive.
    #[error("ram_mib must be greater than zero")]
    ZeroRam,
    /// An explicit max-agent limit must be positive.
    #[error("max_agents must be greater than zero when set")]
    ZeroMaxAgents,
    /// Too many owner rules were supplied.
    #[error("at most 32 owner rules are allowed")]
    TooManyOwnerRules,
    /// An owner rule is empty or exceeds its byte bound.
    #[error("owner rule must contain 1 to {MAX_OWNER_RULE_BYTES} bytes")]
    OwnerRuleLength,
    /// An owner rule contains a character that is unsafe to display.
    #[error("owner rule contains unsafe display text")]
    UnsafeOwnerRule,
}

/// A lead's explicitly reported current focus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusRecord {
    /// The registered lead whose focus was reported.
    pub agent: AgentName,
    /// Short plain-language focus supplied by the operator.
    pub focus: String,
    /// Unix milliseconds when this focus was set.
    pub set_at_ms: i64,
}

/// Explicit capacity entered by an operator for one relay server.
///
/// RAM is measured in mebibytes (MiB). Capacity is never inferred from host
/// telemetry: it is a planning annotation, not a live resource measurement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerCapacity {
    /// Server identity as used in relay addresses.
    pub machine: MachineName,
    /// Number of CPU cores the operator allocates to agent work.
    pub cores: u32,
    /// Allocated memory in mebibytes (MiB).
    pub ram_mib: u64,
    /// Maximum number of agent lanes the operator plans to run here.
    pub max_agents: Option<u32>,
    /// Operator-entered constraints for work on this server.
    pub owner_rules: Vec<String>,
    /// Unix milliseconds when this capacity was set.
    pub set_at_ms: i64,
}

impl ServerCapacity {
    /// Build a capacity record after checking that its numbers and display text
    /// fit the relay's documented bounds. `max_agents: None` means no stated
    /// maximum; it does not cause a capacity estimate to be inferred.
    pub fn try_new(
        machine: MachineName,
        cores: u32,
        ram_mib: u64,
        max_agents: Option<u32>,
        owner_rules: Vec<String>,
        set_at_ms: i64,
    ) -> Result<Self, ServerCapacityError> {
        let record = Self {
            machine,
            cores,
            ram_mib,
            max_agents,
            owner_rules,
            set_at_ms,
        };
        record.validate()?;
        Ok(record)
    }

    /// Validate a record after construction or deserialization.
    pub fn validate(&self) -> Result<(), ServerCapacityError> {
        if self.cores == 0 {
            return Err(ServerCapacityError::ZeroCores);
        }
        if self.ram_mib == 0 {
            return Err(ServerCapacityError::ZeroRam);
        }
        if self.max_agents == Some(0) {
            return Err(ServerCapacityError::ZeroMaxAgents);
        }
        if self.owner_rules.len() > MAX_OWNER_RULES {
            return Err(ServerCapacityError::TooManyOwnerRules);
        }
        for rule in &self.owner_rules {
            if rule.is_empty() || rule.len() > MAX_OWNER_RULE_BYTES {
                return Err(ServerCapacityError::OwnerRuleLength);
            }
            if rule
                .chars()
                .any(|ch| crate::text::is_unsafe_char(ch) || ch == '\n' || ch == '\t')
            {
                return Err(ServerCapacityError::UnsafeOwnerRule);
            }
        }
        Ok(())
    }
}

/// Whether a focus record is older than the configured freshness window.
/// Future timestamps are considered fresh, accommodating small clock skew.
#[must_use]
pub fn focus_is_stale(now_ms: i64, set_at_ms: i64, max_age_ms: u64) -> bool {
    now_ms
        .checked_sub(set_at_ms)
        .is_some_and(|age| age >= 0 && u64::try_from(age).is_ok_and(|age| age > max_age_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trace: FR-057-AC-4, FR-057-AC-5
    #[test]
    fn focus_staleness_marks_old_records_without_hiding_future_records() {
        assert!(!focus_is_stale(10_000, 9_000, 1_000));
        assert!(focus_is_stale(10_001, 9_000, 1_000));
        assert!(!focus_is_stale(9_000, 10_000, 1_000));
    }

    #[test]
    fn capacity_json_names_ram_unit_explicitly() {
        let capacity = ServerCapacity {
            machine: "node-a".parse().expect("valid machine name"),
            cores: 8,
            ram_mib: 16_384,
            max_agents: Some(12),
            owner_rules: vec!["Kani-only".to_owned()],
            set_at_ms: 42,
        };
        let value = serde_json::to_value(capacity).expect("serialize capacity");
        assert_eq!(value["ram_mib"], 16_384);
        assert_eq!(value["max_agents"], 12);
        assert_eq!(value["owner_rules"][0], "Kani-only");
    }

    #[test]
    fn capacity_constructor_accepts_unbounded_agent_limit_and_rules() {
        let capacity = ServerCapacity::try_new(
            "node-a".parse().expect("valid machine name"),
            8,
            16_384,
            None,
            vec!["Kani-only".to_owned()],
            42,
        )
        .expect("valid operator capacity");
        assert_eq!(capacity.max_agents, None);
    }

    #[test]
    fn capacity_rejects_unsafe_or_empty_owner_rules() {
        let make = |rule: &str| {
            ServerCapacity::try_new(
                "node-a".parse().expect("valid machine name"),
                8,
                16_384,
                None,
                vec![rule.to_owned()],
                42,
            )
        };
        assert_eq!(make(""), Err(ServerCapacityError::OwnerRuleLength));
        assert_eq!(
            make("hidden\u{202e}rule"),
            Err(ServerCapacityError::UnsafeOwnerRule)
        );
    }
}
