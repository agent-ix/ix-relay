// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The verified caller of a `focuses` write: the same binding as `agent-msg work` (explicit
//! `--lead` and `--session`, or the verified calling pane). Reads need none.

use relay_core::{
    MachineName, SessionId,
    named_focus::{FocusError, FocusResult, OwnerBinding},
};
use relay_ops::work::resolve_work_target;

/// Establishes who is calling.
pub(crate) trait CallerSource {
    /// The caller's verified live binding at `now_ms`.
    fn verify(
        &self,
        lead: Option<&str>,
        session: Option<&SessionId>,
        now_ms: i64,
    ) -> FocusResult<OwnerBinding>;
}

/// The production caller: measured through Herdr, as `agent-msg work` does.
pub(crate) struct HerdrCaller<'a> {
    pub(crate) machine: &'a MachineName,
}

impl CallerSource for HerdrCaller<'_> {
    fn verify(
        &self,
        lead: Option<&str>,
        session: Option<&SessionId>,
        now_ms: i64,
    ) -> FocusResult<OwnerBinding> {
        let observation = resolve_work_target(&herdr_cli::RealProcessRunner, lead, self.machine)
            .map_err(|error| {
                FocusError::invalid(
                    "lead",
                    format!("the calling session could not be verified: {error}"),
                )
            })?;
        let binding = observation.binding;
        if session.is_some_and(|expected| *expected != binding.session) {
            return Err(FocusError::invalid(
                "session",
                "the pane hosts a different native session than --session names",
            ));
        }
        Ok(OwnerBinding::new(
            &binding.machine,
            &binding.session,
            Some(binding.process_id),
            &binding.address,
            now_ms,
        ))
    }
}
