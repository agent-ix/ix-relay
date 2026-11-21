// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Agent-IX
//! Classification and wording of the ix-board service's typed quota metadata for the
//! dashboard's tracker notices.
//!
//! Everything here reads typed fields: the refresh outcome, the quota decision and the SDK's
//! typed errors. No message text is parsed, no failure reason is classified and no transport
//! detail is shown. Only [`Cause::ServerBackoff`] is a server rate limit, an actual HTTP 429
//! or GraphQL `RATELIMITED` cooldown. Every other hold is this tool's own protection, an
//! unreadable ledger or a refused request, and is worded as such. Times in a decision are
//! Unix milliseconds; they are shown only relative to the attempt that captured them.

use ix_board::quota::{QuotaBlocker, QuotaDecision, QuotaDimension, QuotaState};

/// Why the latest tracker attempt did not produce a fresh snapshot.
///
/// One closed variant per typed SDK state or outcome. A variant is never reconstructed from
/// message text, so an attempt whose typed outcome is absent stays [`Cause::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// An actual server cooldown: the decision state `ServerBackoff`, or the SDK's legacy typed
    /// `LinearRateLimited` error. The only cause that is a server rate limit.
    ServerBackoff,
    /// The fixed local allocation is exhausted. Not a server limit.
    LocalWindow,
    /// A locally enforced reserve against a live server observation. Not a server limit.
    ServerReserve,
    /// The SDK's legacy typed `LinearLocalBudgetExhausted` error, which does not say which
    /// local hold applied. Not a server limit.
    LocalBudget,
    /// The persisted quota evidence cannot be interpreted. Not a server limit.
    EvidenceUnavailable,
    /// As [`Cause::EvidenceUnavailable`], with pre-fence reservations still pending
    /// reconciliation by the quota owner. Not a server limit.
    EvidenceQuarantined,
    /// Waiting cannot admit this request or configuration.
    RequestRejected,
    /// Admission was refused but the decision reports no hold.
    RefusalUnexplained,
    /// The shared refresh lock could not be acquired.
    RefreshLockBusy,
    /// The source or transport failed for a reason that is not quota; no cause is shown.
    SourceFailed,
    /// The refresh did not complete and the service reports no typed outcome.
    Unknown,
}

impl Cause {
    /// The cause of a refused admission, from its primary typed state.
    pub fn of_decision(decision: &QuotaDecision) -> Self {
        match decision.state {
            QuotaState::ServerBackoff => Self::ServerBackoff,
            QuotaState::LocalWindowHeld => Self::LocalWindow,
            QuotaState::ServerReserveHeld => Self::ServerReserve,
            QuotaState::EvidenceUnavailable if decision.legacy_pending_reservations > 0 => {
                Self::EvidenceQuarantined
            }
            QuotaState::EvidenceUnavailable => Self::EvidenceUnavailable,
            QuotaState::RequestRejected => Self::RequestRejected,
            QuotaState::Available => Self::RefusalUnexplained,
        }
    }

    /// The quota cause of a failed read with no snapshot, or `None` when the failure is not a
    /// quota matter (authentication, transport and the rest keep their own classification).
    pub fn of_error(error: &ix_board::Error) -> Option<Self> {
        match error {
            ix_board::Error::LinearRateLimited { .. } => Some(Self::ServerBackoff),
            ix_board::Error::LinearLocalBudgetExhausted { .. } => Some(Self::LocalBudget),
            other => other.quota_decision().map(Self::of_decision),
        }
    }

    /// Compact one-line statement of the cause.
    pub fn summary(self) -> &'static str {
        match self {
            Self::ServerBackoff => "Linear rate limited",
            Self::LocalWindow => "Linear local allocation held",
            Self::ServerReserve => "Linear server reserve held",
            Self::LocalBudget => "Linear request budget exhausted",
            Self::EvidenceUnavailable => "Linear quota evidence unavailable",
            Self::EvidenceQuarantined => "Linear quota evidence quarantined",
            Self::RequestRejected => "Linear request cannot be admitted",
            Self::RefusalUnexplained => "Linear quota refused the request; no hold reported",
            Self::RefreshLockBusy => "Linear refresh lock busy",
            Self::SourceFailed | Self::Unknown => "Linear refresh failed",
        }
    }

    /// Evidence-row label; it stays short because labels are never truncated.
    pub fn label(self) -> &'static str {
        match self {
            Self::ServerBackoff => "Tracker rate-limited",
            Self::LocalWindow => "Tracker held: local allocation",
            Self::ServerReserve => "Tracker held: server reserve",
            Self::LocalBudget => "Tracker held: request budget",
            Self::EvidenceUnavailable => "Tracker quota evidence unavailable",
            Self::EvidenceQuarantined => "Tracker quota evidence quarantined",
            Self::RequestRejected => "Tracker request cannot be admitted",
            Self::RefusalUnexplained => "Tracker quota refused",
            Self::RefreshLockBusy => "Tracker refresh lock busy",
            Self::SourceFailed | Self::Unknown => "Tracker failed",
        }
    }

    /// Whether the cause is a hold to wait out: the next attempt keeps its scheduled time and
    /// manual refreshes coalesce until then. Waiting does not guarantee admission.
    pub fn waits(self) -> bool {
        match self {
            Self::ServerBackoff | Self::LocalWindow | Self::ServerReserve | Self::LocalBudget => {
                true
            }
            Self::EvidenceUnavailable
            | Self::EvidenceQuarantined
            | Self::RequestRejected
            | Self::RefusalUnexplained
            | Self::RefreshLockBusy
            | Self::SourceFailed
            | Self::Unknown => false,
        }
    }

    /// What the reader can do about a cause that waiting does not clear.
    fn remedy(self) -> Option<&'static str> {
        match self {
            Self::ServerBackoff
            | Self::LocalWindow
            | Self::ServerReserve
            | Self::LocalBudget
            | Self::RefreshLockBusy => None,
            Self::EvidenceUnavailable | Self::EvidenceQuarantined => {
                Some("Ask the quota owner to reconcile the quota evidence, or open --report")
            }
            Self::RequestRejected => {
                Some("Waiting will not admit this request; inspect ix-board, or open --report")
            }
            Self::RefusalUnexplained | Self::SourceFailed | Self::Unknown => {
                Some(super::SOURCE_AVAILABILITY_REMEDY)
            }
        }
    }
}

/// Text explaining one attempt that did not produce a fresh snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The primary statement: what held or failed, and the kept snapshot when there is one.
    pub summary: String,
    /// The summary followed by every fact the service reported: the attempt, the earliest
    /// reevaluation and every blocker, then what the reader can do.
    pub detail: String,
}

impl Notice {
    /// Wording for `cause`; `decision` is the typed decision when the service captured one and
    /// `snapshot_kept` says whether the last snapshot is still shown.
    pub fn new(cause: Cause, decision: Option<&QuotaDecision>, snapshot_kept: bool) -> Self {
        let summary = if snapshot_kept {
            format!("{}; showing the last snapshot", cause.summary())
        } else {
            cause.summary().to_owned()
        };
        let detail = match cause {
            Cause::SourceFailed | Cause::Unknown => super::render_diagnostic(
                "TrackerRefreshFailed",
                &summary,
                super::SOURCE_AVAILABILITY_REMEDY,
            ),
            _ => {
                let mut parts = vec![summary.clone()];
                if let Some(decision) = decision {
                    parts.extend(decision_facts(decision));
                }
                parts.extend(cause.remedy().map(str::to_owned));
                parts.join(" · ")
            }
        };
        Self { summary, detail }
    }
}

/// Every reported fact of a decision: the attempt, the earliest reevaluation and each blocker.
fn decision_facts(decision: &QuotaDecision) -> Vec<String> {
    let mut facts = vec![format!(
        "attempt {} with estimated complexity {}",
        decision.endpoint, decision.estimated_complexity
    )];
    if decision.legacy_pending_reservations > 0 {
        facts.push(format!(
            "{} pre-fence reservations pending reconciliation; not a server limit",
            decision.legacy_pending_reservations
        ));
    }
    facts.push(match decision.next_reevaluation_at {
        Some(at) => format!(
            "earliest reevaluation {} after the attempt; admission is not guaranteed",
            span(at.saturating_sub(decision.evaluated_at))
        ),
        None => "reevaluation time unknown".to_owned(),
    });
    facts.extend(
        decision
            .blockers
            .iter()
            .map(|blocker| format!("hold: {}", blocker_text(blocker, decision.evaluated_at))),
    );
    if !decision.evidence.is_empty() {
        facts.push(format!(
            "{} live server observations reported",
            decision.evidence.len()
        ));
    }
    facts
}

/// One blocker with the values the service reported for it; absent values are not invented.
fn blocker_text(blocker: &QuotaBlocker, evaluated_at: i64) -> String {
    let name = match blocker.dimension {
        Some(dimension) => format!(
            "{} {}",
            state_label(blocker.state),
            dimension_label(dimension)
        ),
        None => state_label(blocker.state).to_owned(),
    };
    let mut values = Vec::new();
    for (label, value) in [
        ("capacity", blocker.capacity),
        ("reserve", blocker.reserve),
        ("charge", blocker.estimated_charge),
        ("local usage", blocker.local_usage),
        ("outstanding", blocker.outstanding_charge),
    ] {
        if let Some(value) = value {
            values.push(format!("{label} {value}"));
        }
    }
    // A reserve hold is a statement about what the server has left; absence means unknown.
    match (blocker.effective_remaining, blocker.state) {
        (Some(remaining), _) => values.push(format!("remaining {remaining}")),
        (None, QuotaState::ServerReserveHeld) => values.push("remaining unknown".to_owned()),
        (None, _) => {}
    }
    values.push(match blocker.deadline {
        Some(deadline) => format!(
            "reevaluate after {}",
            span(deadline.saturating_sub(evaluated_at))
        ),
        None => "no reevaluation time".to_owned(),
    });
    format!("{name} ({})", values.join(", "))
}

fn state_label(state: QuotaState) -> &'static str {
    match state {
        QuotaState::Available => "available",
        QuotaState::LocalWindowHeld => "local allocation held",
        QuotaState::ServerReserveHeld => "server reserve held",
        QuotaState::ServerBackoff => "server backoff",
        QuotaState::EvidenceUnavailable => "evidence unavailable",
        QuotaState::RequestRejected => "request cannot be admitted",
    }
}

fn dimension_label(dimension: QuotaDimension) -> &'static str {
    match dimension {
        QuotaDimension::Requests => "requests",
        QuotaDimension::Complexity => "complexity",
        QuotaDimension::EndpointRequests => "endpoint requests",
    }
}

/// An interval in milliseconds rounded up to whole seconds, such as `12m 03s`; a negative
/// interval is shown as `0s`.
fn span(milliseconds: i64) -> String {
    let total = u64::try_from(milliseconds).unwrap_or(0).div_ceil(1000);
    match (total / 3600, total % 3600 / 60, total % 60) {
        (0, 0, seconds) => format!("{seconds}s"),
        (0, minutes, seconds) => format!("{minutes}m {seconds:02}s"),
        (hours, minutes, _) => format!("{hours}h {minutes:02}m"),
    }
}

/// Typed decision fixtures shared by the dashboard's tests. Their numbers are arbitrary: the
/// tests assert that the dashboard shows what a decision reports, never a production value.
#[cfg(test)]
pub(crate) mod fixture {
    use super::{QuotaBlocker, QuotaDecision, QuotaDimension, QuotaState};

    /// When every fixture decision was captured, in Unix milliseconds.
    pub(crate) const EVALUATED_AT: i64 = 1_000_000;

    /// A decision captured at [`EVALUATED_AT`]. As the SDK derives them, its primary state is
    /// the first blocker's and its earliest reevaluation is the earliest blocker deadline.
    pub(crate) fn decision(blockers: Vec<QuotaBlocker>) -> QuotaDecision {
        QuotaDecision {
            state: blockers
                .first()
                .map_or(QuotaState::Available, |blocker| blocker.state),
            next_reevaluation_at: blockers.iter().filter_map(|blocker| blocker.deadline).min(),
            blockers,
            evidence: Vec::new(),
            evaluated_at: EVALUATED_AT,
            estimated_complexity: 10_000,
            endpoint: "graphql".to_owned(),
            legacy_pending_reservations: 0,
            local_generation: 3,
            local_window_reset_at: EVALUATED_AT + 3_000_000,
        }
    }

    /// A blocker that reports only its state, dimension and absolute deadline.
    pub(crate) fn blocker(
        state: QuotaState,
        dimension: Option<QuotaDimension>,
        deadline: Option<i64>,
    ) -> QuotaBlocker {
        QuotaBlocker {
            state,
            dimension,
            deadline,
            capacity: None,
            reserve: None,
            estimated_charge: None,
            local_usage: None,
            outstanding_charge: None,
            effective_remaining: None,
            observation_id: None,
            observed_at: None,
            paired_reset: false,
        }
    }

    /// A server reserve hold on requests while local usage is zero; it reevaluates in 12m 03s.
    pub(crate) fn reserve_held() -> QuotaDecision {
        let mut held = blocker(
            QuotaState::ServerReserveHeld,
            Some(QuotaDimension::Requests),
            Some(EVALUATED_AT + 723_000),
        );
        held.capacity = Some(40);
        held.reserve = Some(7);
        held.estimated_charge = Some(1);
        held.local_usage = Some(0);
        held.outstanding_charge = Some(0);
        held.effective_remaining = Some(3);
        decision(vec![held])
    }

    /// An exhausted local complexity allocation; it reevaluates at the local window reset.
    pub(crate) fn local_held() -> QuotaDecision {
        let mut held = blocker(
            QuotaState::LocalWindowHeld,
            Some(QuotaDimension::Complexity),
            Some(EVALUATED_AT + 3_000_000),
        );
        held.capacity = Some(900);
        held.local_usage = Some(880);
        held.estimated_charge = Some(50);
        decision(vec![held])
    }

    /// An actual server cooldown, reevaluating in 1m 30s.
    pub(crate) fn server_backoff() -> QuotaDecision {
        decision(vec![blocker(
            QuotaState::ServerBackoff,
            None,
            Some(EVALUATED_AT + 90_000),
        )])
    }

    /// Unreadable quota evidence with two pre-fence reservations pending and no known time.
    pub(crate) fn quarantined() -> QuotaDecision {
        let mut held = decision(vec![blocker(QuotaState::EvidenceUnavailable, None, None)]);
        held.legacy_pending_reservations = 2;
        held
    }

    /// Three independent holds at once; the primary state is evidence unavailable.
    pub(crate) fn simultaneous() -> QuotaDecision {
        let mut held = decision(vec![
            blocker(QuotaState::EvidenceUnavailable, None, None),
            blocker(
                QuotaState::LocalWindowHeld,
                Some(QuotaDimension::Requests),
                Some(EVALUATED_AT + 2_000_000),
            ),
            blocker(
                QuotaState::ServerReserveHeld,
                Some(QuotaDimension::EndpointRequests),
                Some(EVALUATED_AT + 500_000),
            ),
        ]);
        held.legacy_pending_reservations = 4;
        held
    }
}

#[cfg(test)]
mod tests {
    use ix_trace_rs::trace;

    use super::fixture::{blocker, decision, local_held, quarantined, reserve_held, simultaneous};
    use super::*;

    /// Every decision state maps to its own cause, and only a server backoff is a rate limit.
    #[trace("FR-048-AC-5")]
    #[test]
    fn each_decision_state_maps_to_one_cause_and_only_backoff_is_a_rate_limit() {
        let cause = |state, legacy| {
            let mut decision = decision(vec![blocker(state, None, None)]);
            decision.legacy_pending_reservations = legacy;
            Cause::of_decision(&decision)
        };
        assert_eq!(cause(QuotaState::ServerBackoff, 0), Cause::ServerBackoff);
        assert_eq!(cause(QuotaState::LocalWindowHeld, 0), Cause::LocalWindow);
        assert_eq!(
            cause(QuotaState::ServerReserveHeld, 0),
            Cause::ServerReserve
        );
        assert_eq!(
            cause(QuotaState::EvidenceUnavailable, 0),
            Cause::EvidenceUnavailable
        );
        assert_eq!(
            cause(QuotaState::EvidenceUnavailable, 2),
            Cause::EvidenceQuarantined
        );
        assert_eq!(
            cause(QuotaState::RequestRejected, 0),
            Cause::RequestRejected
        );
        assert_eq!(cause(QuotaState::Available, 0), Cause::RefusalUnexplained);
        for held in [
            Cause::LocalWindow,
            Cause::ServerReserve,
            Cause::LocalBudget,
            Cause::EvidenceUnavailable,
            Cause::EvidenceQuarantined,
            Cause::RequestRejected,
            Cause::RefusalUnexplained,
        ] {
            let text = Notice::new(held, None, false).detail.to_lowercase();
            assert!(!text.contains("rate"), "{held:?}: {text}");
            assert!(!text.contains("429"), "{held:?}: {text}");
        }
    }

    /// A server reserve hold with no local usage still reports the reserve, the charge and the
    /// remaining the service gave; nothing is attributed to the local allocation.
    #[trace("FR-048-AC-5")]
    #[test]
    fn a_reserve_hold_with_no_local_usage_reports_its_own_values() {
        let decision = reserve_held();
        let notice = Notice::new(Cause::of_decision(&decision), Some(&decision), true);
        assert_eq!(
            notice.summary,
            "Linear server reserve held; showing the last snapshot"
        );
        for fact in [
            "server reserve held requests",
            "capacity 40",
            "reserve 7",
            "charge 1",
            "local usage 0",
            "remaining 3",
            "reevaluate after 12m 03s",
            "earliest reevaluation 12m 03s after the attempt; admission is not guaranteed",
        ] {
            assert!(notice.detail.contains(fact), "{fact}: {}", notice.detail);
        }
        assert!(
            !notice.detail.contains("local allocation"),
            "{}",
            notice.detail
        );
    }

    /// A local window hold is its own cause, with its own exhausted usage.
    #[trace("FR-048-AC-5")]
    #[test]
    fn a_local_window_hold_reports_the_exhausted_local_usage() {
        let decision = local_held();
        let notice = Notice::new(Cause::of_decision(&decision), Some(&decision), false);
        assert_eq!(notice.summary, "Linear local allocation held");
        assert!(notice.detail.contains("local allocation held complexity"));
        assert!(notice.detail.contains("local usage 880"));
        assert!(notice.detail.contains("capacity 900"));
        assert!(notice.detail.contains("reevaluate after 50m 00s"));
        assert!(
            !notice.detail.contains("server reserve"),
            "{}",
            notice.detail
        );
    }

    /// Simultaneous blockers all stay in the detail, in reported order, and the primary summary
    /// names only the decision's primary state.
    #[trace("FR-048-AC-5")]
    #[test]
    fn simultaneous_blockers_are_all_reported_under_the_primary_summary() {
        let decision = simultaneous();
        let notice = Notice::new(Cause::of_decision(&decision), Some(&decision), false);
        assert_eq!(notice.summary, "Linear quota evidence quarantined");
        let evidence = notice.detail.find("evidence unavailable (").unwrap();
        let local = notice
            .detail
            .find("local allocation held requests")
            .unwrap();
        let reserve = notice
            .detail
            .find("server reserve held endpoint requests")
            .unwrap();
        assert!(evidence < local && local < reserve, "{}", notice.detail);
        assert!(notice.detail.contains("no reevaluation time"));
        assert!(notice.detail.contains("4 pre-fence reservations pending"));
        assert!(notice.detail.contains("earliest reevaluation 8m 20s after"));
        assert!(notice.detail.contains("reconcile the quota evidence"));
    }

    /// No reevaluation time stays unknown: no countdown and no timer is invented.
    #[trace("FR-048-AC-5")]
    #[test]
    fn an_unknown_reevaluation_time_is_reported_as_unknown() {
        let decision = quarantined();
        assert_eq!(decision.next_reevaluation_at, None);
        let notice = Notice::new(Cause::of_decision(&decision), Some(&decision), false);
        assert!(notice.detail.contains("reevaluation time unknown"));
        assert!(!notice.detail.contains("earliest reevaluation"));
        assert!(notice.detail.contains("2 pre-fence reservations pending"));
        assert!(!Cause::of_decision(&decision).waits());
    }

    /// A refresh failure with no quota decision names no cause and carries no raw detail.
    #[trace("FR-059-AC-5")]
    #[test]
    fn a_nonquota_failure_names_no_cause() {
        for cause in [Cause::SourceFailed, Cause::Unknown] {
            let notice = Notice::new(cause, None, true);
            assert_eq!(
                notice.summary,
                "Linear refresh failed; showing the last snapshot"
            );
            assert!(notice.detail.contains("Linear refresh failed"));
            assert!(!cause.waits());
        }
    }

    /// Only the holds to wait out coalesce manual refreshes; a rate limit is only a backoff.
    #[trace("FR-048-AC-5")]
    #[test]
    fn only_waiting_holds_coalesce_and_only_backoff_is_labelled_rate_limited() {
        let all = [
            Cause::ServerBackoff,
            Cause::LocalWindow,
            Cause::ServerReserve,
            Cause::LocalBudget,
            Cause::EvidenceUnavailable,
            Cause::EvidenceQuarantined,
            Cause::RequestRejected,
            Cause::RefusalUnexplained,
            Cause::RefreshLockBusy,
            Cause::SourceFailed,
            Cause::Unknown,
        ];
        let waiting = all
            .into_iter()
            .filter(|cause| cause.waits())
            .collect::<Vec<_>>();
        assert_eq!(
            waiting,
            [
                Cause::ServerBackoff,
                Cause::LocalWindow,
                Cause::ServerReserve,
                Cause::LocalBudget
            ]
        );
        for cause in all {
            let rate_limited =
                cause.label().contains("rate-limited") || cause.summary().contains("rate limited");
            assert_eq!(rate_limited, cause == Cause::ServerBackoff, "{cause:?}");
        }
    }

    #[test]
    fn spans_round_up_to_whole_seconds_and_never_go_negative() {
        assert_eq!(span(-5), "0s");
        assert_eq!(span(1), "1s");
        assert_eq!(span(59_000), "59s");
        assert_eq!(span(723_000), "12m 03s");
        assert_eq!(span(3_660_000), "1h 01m");
    }
}
