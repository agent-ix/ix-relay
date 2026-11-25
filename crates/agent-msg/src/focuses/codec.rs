// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The stored selection document, through ix-board's own encoder and decoder.
//!
//! ix-board owns the version 3 named-selection document: its member shapes, its canonical
//! order and its encoding. relay-core stores and reads the document only through the
//! [`SelectionCodec`] port, so this is the one place the two meet, and nothing here spells
//! a kind, a key or a version. The relay-core integration tests include this file as it is,
//! so they exercise the same mapping the command runs.

use ix_board::{Error, NamedMember, NamedSelection, TeamKey, decode_named_selection};
use relay_core::named_focus::{
    DecodedSelection, FocusError, FocusResult, Member, MemberKind, SelectionCodec,
    UnavailableReason,
};

/// The codec of the pinned ix-board.
pub(crate) struct IxBoardCodec;

/// The ix-board member of a relay member.
fn named(member: &Member) -> NamedMember {
    let id = member.id.clone();
    match member.kind {
        MemberKind::Initiative => NamedMember::Initiative { id },
        MemberKind::Project => NamedMember::Project { id },
        MemberKind::Milestone => NamedMember::Milestone {
            id,
            project_id: member.project_id.clone().unwrap_or_default(),
        },
        MemberKind::Issue => NamedMember::Issue { id },
    }
}

/// The relay member of an ix-board member, with no label.
fn member(named: &NamedMember) -> Member {
    match named {
        NamedMember::Initiative { id } => Member::new(MemberKind::Initiative, id, ""),
        NamedMember::Project { id } => Member::new(MemberKind::Project, id, ""),
        NamedMember::Milestone { id, project_id } => Member::milestone(id, project_id, ""),
        NamedMember::Issue { id } => Member::new(MemberKind::Issue, id, ""),
    }
}

impl SelectionCodec for IxBoardCodec {
    fn encode(&self, team: &str, include: &[Member], exclude: &[Member]) -> FocusResult<String> {
        NamedSelection::new(
            TeamKey(team.to_owned()),
            include.iter().map(named).collect(),
            exclude.iter().map(named).collect(),
        )
        .encode()
        .map_err(|error| FocusError::invalid("selection", error.to_string()))
    }

    fn decode(&self, document: &[u8]) -> Result<DecodedSelection, UnavailableReason> {
        match decode_named_selection(document) {
            Ok(selection) => Ok(DecodedSelection {
                team: selection.team.0,
                include: selection.include.iter().map(member).collect(),
                exclude: selection.exclude.iter().map(member).collect(),
            }),
            Err(Error::UnsupportedSchemaVersion { .. }) => {
                Err(UnavailableReason::UnsupportedSchema)
            }
            Err(_) => Err(UnavailableReason::Undecodable),
        }
    }
}
