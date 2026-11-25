// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The stored shapes of a named focus definition: identity, content, revisions, owner, pins.
//!
//! Stored records are decoded tolerantly (FR-061 "Stored-record tolerance"): the records
//! the relay rewrites (`meta`, `owner`, a pin) keep every member they do not define, so a
//! newer machine's addition survives an older machine's update. Revisions are immutable and
//! never rewritten.

use std::{collections::BTreeMap, fmt, fs::File, io::Read, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, value::RawValue};

use super::{
    error::{FocusError, FocusResult, UnavailableReason},
    limits::{
        MAX_DESCRIPTION_BYTES, MAX_MEMBER_LABEL_BYTES, MAX_NAME_BYTES, MAX_REVISION_BYTES,
        MAX_SELECTOR_BYTES, MAX_SELECTORS,
    },
};
use crate::{
    ids::{MachineName, SessionId},
    text,
};

/// What every focus ID starts with.
pub const FOCUS_ID_PREFIX: &str = "foc-";

/// The opaque, never-reused identity of a definition: `foc-` and 32 lowercase hexadecimal
/// characters from 128 random bits.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FocusId(String);

impl FocusId {
    /// A fresh ID from 128 random bits of the operating system.
    ///
    /// # Errors
    ///
    /// [`FocusError::Random`] when `/dev/urandom` cannot be read; no weaker source is used.
    pub fn generate() -> FocusResult<Self> {
        let mut bytes = [0_u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut bytes))
            .map_err(|error| FocusError::Random(error.to_string()))?;
        let mut id = String::from(FOCUS_ID_PREFIX);
        for byte in bytes {
            id.push_str(&format!("{byte:02x}"));
        }
        Ok(Self(id))
    }

    /// The ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `text` has the shape of a focus ID.
    #[must_use]
    pub fn is_well_formed(text: &str) -> bool {
        text.len() == FOCUS_ID_PREFIX.len() + 32
            && text.starts_with(FOCUS_ID_PREFIX)
            && text
                .bytes()
                .skip(FOCUS_ID_PREFIX.len())
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }
}

impl FromStr for FocusId {
    type Err = FocusError;

    fn from_str(text: &str) -> FocusResult<Self> {
        if Self::is_well_formed(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(FocusError::invalid(
                "focus",
                format!(
                    "{text:?} is not a focus ID (`foc-` and 32 lowercase hexadecimal characters)"
                ),
            ))
        }
    }
}

impl TryFrom<String> for FocusId {
    type Error = FocusError;

    fn try_from(text: String) -> FocusResult<Self> {
        text.parse()
    }
}

impl From<FocusId> for String {
    fn from(id: FocusId) -> Self {
        id.0
    }
}

impl fmt::Display for FocusId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The four kinds of selection member, in the canonical order of the shared selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemberKind {
    /// A Linear initiative.
    Initiative,
    /// A Linear project.
    Project,
    /// A project milestone, recorded with its owning project.
    Milestone,
    /// A ticket, covering its descendants.
    Issue,
}

impl MemberKind {
    /// The kind as printed for a person. Not the stored document's spelling, which only the
    /// [`SelectionCodec`] knows.
    #[must_use]
    pub const fn noun(self) -> &'static str {
        match self {
            Self::Initiative => "initiative",
            Self::Project => "project",
            Self::Milestone => "milestone",
            Self::Issue => "ticket",
        }
    }
}

/// One canonical selection member with its display label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// What kind of Linear object.
    pub kind: MemberKind,
    /// The object's canonical stable ID.
    pub id: String,
    /// The owning project's stable ID; present for a milestone and for nothing else.
    pub project_id: Option<String>,
    /// A display label, used only for display.
    pub label: String,
}

impl Member {
    /// A member of `kind` with `id` and `label`, and no project.
    #[must_use]
    pub fn new(kind: MemberKind, id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            project_id: None,
            label: label.into(),
        }
    }

    /// A milestone member with its owning project.
    #[must_use]
    pub fn milestone(
        id: impl Into<String>,
        project_id: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            kind: MemberKind::Milestone,
            id: id.into(),
            project_id: Some(project_id.into()),
            label: label.into(),
        }
    }

    fn identity(&self) -> (MemberKind, &str, Option<&str>) {
        (self.kind, &self.id, self.project_id.as_deref())
    }
}

/// What a caller supplies to create a definition or to replace its content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusContent {
    /// The one Linear team key the definition belongs to.
    pub team: String,
    /// The editable name.
    pub name: String,
    /// The description, empty when there is none.
    pub description: String,
    /// What the focus selects; empty selects nothing.
    pub include: Vec<Member>,
    /// What the focus removes from its own selection.
    pub exclude: Vec<Member>,
}

/// Bytes cut so that no UTF-8 sequence is split.
pub(crate) fn cut_utf8(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// Free text stored as given after the relay's own text rules (FR-061: a name is sanitized
/// as a single line like a work-report field; a description keeps its line breaks and
/// loses only unsafe characters). A text with nothing in it is refused when `required`.
fn sanitized_text(
    field: &str,
    value: &str,
    (bound, max): (&'static str, usize),
    single_line: bool,
    required: bool,
) -> FocusResult<String> {
    let stored = if single_line {
        text::sanitise_line(value)
    } else {
        text::sanitise(value).into_owned()
    };
    if required && stored.trim().is_empty() {
        return Err(FocusError::invalid(field, "must not be empty"));
    }
    if stored.len() > max {
        return Err(FocusError::limit(bound, max));
    }
    Ok(stored)
}

/// An identifier or reference: refused, never rewritten, when it is not one line of safe
/// text, because a changed identifier would name a different object.
fn checked_line(
    field: &str,
    value: String,
    (bound, max): (&'static str, usize),
    required: bool,
) -> FocusResult<String> {
    if required && value.trim().is_empty() {
        return Err(FocusError::invalid(field, "must not be empty"));
    }
    if value != text::sanitise_line(&value) {
        return Err(FocusError::invalid(
            field,
            "must be one line of plain text without control characters",
        ));
    }
    if value.len() > max {
        return Err(FocusError::limit(bound, max));
    }
    Ok(value)
}

const SELECTOR_BOUND: (&str, usize) = ("bytes per stable ID or selector", MAX_SELECTOR_BYTES);

fn normalize_members(list: &str, members: Vec<Member>) -> FocusResult<Vec<Member>> {
    type Key = (MemberKind, String, Option<String>);
    let mut seen: BTreeMap<Key, String> = BTreeMap::new();
    for member in members {
        let id = checked_line(
            &format!("{list} member id"),
            member.id,
            SELECTOR_BOUND,
            true,
        )?;
        let project = match (member.kind, member.project_id) {
            (MemberKind::Milestone, Some(project)) => Some(checked_line(
                &format!("{list} milestone project id"),
                project,
                SELECTOR_BOUND,
                true,
            )?),
            (MemberKind::Milestone, None) => {
                return Err(FocusError::invalid(
                    format!("{list} milestone"),
                    "a milestone member records its owning project ID",
                ));
            }
            (kind, Some(_)) => {
                return Err(FocusError::invalid(
                    format!("{list} {}", kind.noun()),
                    "only a milestone member has a project ID",
                ));
            }
            (_, None) => None,
        };
        let line = text::sanitise_line(&member.label);
        let label = cut_utf8(&line, MAX_MEMBER_LABEL_BYTES);
        let stored = seen.entry((member.kind, id, project)).or_default();
        if stored.is_empty() {
            label.clone_into(stored);
        }
    }
    Ok(seen
        .into_iter()
        .map(|((kind, id, project_id), label)| Member {
            kind,
            id,
            project_id,
            label,
        })
        .collect())
}

impl FocusContent {
    /// The content in its stored form: a sanitized single-line name, a sanitized description,
    /// members in canonical order without duplicates, labels cut to 64 bytes on a UTF-8
    /// boundary.
    ///
    /// # Errors
    ///
    /// [`FocusError::InvalidArgument`] for an empty name or team, an identifier that is not
    /// one line of safe text, or a malformed member, and [`FocusError::Limit`] naming the bound for an over-long field or too many
    /// selectors. Nothing is stored by this call.
    pub fn normalize(self) -> FocusResult<Self> {
        let team = checked_line(
            "team",
            self.team,
            ("bytes per stable ID or selector", MAX_SELECTOR_BYTES),
            true,
        )?;
        let name = sanitized_text(
            "name",
            &self.name,
            ("name bytes", MAX_NAME_BYTES),
            true,
            true,
        )?;
        let description = sanitized_text(
            "description",
            &self.description,
            ("description bytes", MAX_DESCRIPTION_BYTES),
            false,
            false,
        )?;
        let include = normalize_members("include", self.include)?;
        let exclude = normalize_members("exclude", self.exclude)?;
        if include.len() + exclude.len() > MAX_SELECTORS {
            return Err(FocusError::limit("selectors per revision", MAX_SELECTORS));
        }
        Ok(Self {
            team,
            name,
            description,
            include,
            exclude,
        })
    }
}

/// A selection as the codec reads it: the team and the members of each list in the codec's
/// canonical order. The members carry no label; a label is the relay's own display metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedSelection {
    /// The team the selection is fenced to.
    pub team: String,
    /// What is included.
    pub include: Vec<Member>,
    /// What is removed from the selection.
    pub exclude: Vec<Member>,
}

/// The one place that knows the stored selection document.
///
/// A revision embeds the shared ix-board selection, and ix-board owns its wire type, its
/// version and its canonical encoding. The relay never spells that document: its caller
/// supplies this codec, built on the ix-board encoder and decoder, and the relay stores the
/// bytes `encode` returns and reads them back only through `decode`.
pub trait SelectionCodec {
    /// The canonical document of `team`, `include` and `exclude`, exactly as ix-board encodes it.
    ///
    /// # Errors
    ///
    /// [`FocusError::InvalidArgument`] when the document cannot be encoded.
    fn encode(&self, team: &str, include: &[Member], exclude: &[Member]) -> FocusResult<String>;

    /// The selection a stored document holds.
    ///
    /// # Errors
    ///
    /// [`UnavailableReason::UnsupportedSchema`] for a version this build does not read,
    /// [`UnavailableReason::Undecodable`] for a document that is not a valid selection.
    fn decode(&self, document: &[u8]) -> Result<DecodedSelection, UnavailableReason>;
}

/// Display labels of a revision's members, in the order the codec decodes each list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labels {
    /// One label per inclusion.
    #[serde(default)]
    pub include: Vec<String>,
    /// One label per exclusion.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// One immutable revision of a definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Revision {
    /// The definition.
    pub focus_id: FocusId,
    /// The revision number, from 1.
    pub revision: u64,
    /// The team.
    pub team: String,
    /// The name at this revision.
    pub name: String,
    /// The description at this revision.
    pub description: String,
    /// The embedded shared ix-board selection, byte for byte as the codec encoded it:
    /// canonical IDs only.
    pub selection: Box<RawValue>,
    /// Display labels of the selection's members.
    #[serde(default)]
    pub labels: Labels,
    /// The owner epoch at write time.
    pub owner_epoch: u64,
    /// The write time, Unix milliseconds.
    pub written_at_ms: i64,
}

impl PartialEq for Revision {
    fn eq(&self, other: &Self) -> bool {
        self.focus_id == other.focus_id
            && self.revision == other.revision
            && self.team == other.team
            && self.name == other.name
            && self.description == other.description
            && self.selection.get() == other.selection.get()
            && self.labels == other.labels
            && self.owner_epoch == other.owner_epoch
            && self.written_at_ms == other.written_at_ms
    }
}

/// The label of the member of `members` that has `member`'s identity, or none.
fn label_of(members: &[Member], member: &Member) -> String {
    members
        .iter()
        .find(|candidate| candidate.identity() == member.identity())
        .map(|found| found.label.clone())
        .unwrap_or_default()
}

impl Revision {
    /// Builds revision `number` of `focus` from normalized `content`.
    ///
    /// The selection is the codec's own encoding; the labels are lined up with the order in
    /// which the codec reads that document back, so they follow whatever canonical order
    /// ix-board uses.
    ///
    /// # Errors
    ///
    /// [`FocusError::InvalidArgument`] when the codec cannot encode the selection or reads
    /// back a different one.
    pub fn build(
        codec: &dyn SelectionCodec,
        focus: &FocusId,
        number: u64,
        content: &FocusContent,
        owner_epoch: u64,
        written_at_ms: i64,
    ) -> FocusResult<Self> {
        let document = codec.encode(&content.team, &content.include, &content.exclude)?;
        let selection = RawValue::from_string(document)
            .map_err(|error| FocusError::invalid("selection", error.to_string()))?;
        let read = codec
            .decode(selection.get().as_bytes())
            .map_err(|reason| FocusError::invalid("selection", reason.to_string()))?;
        let labels = Labels {
            include: read
                .include
                .iter()
                .map(|member| label_of(&content.include, member))
                .collect(),
            exclude: read
                .exclude
                .iter()
                .map(|member| label_of(&content.exclude, member))
                .collect(),
        };
        Ok(Self {
            focus_id: focus.clone(),
            revision: number,
            team: content.team.clone(),
            name: content.name.clone(),
            description: content.description.clone(),
            selection,
            labels,
            owner_epoch,
            written_at_ms,
        })
    }

    /// The revision as stored.
    ///
    /// # Errors
    ///
    /// [`FocusError::Limit`] when the encoding exceeds 56 KiB.
    pub fn encode(&self) -> FocusResult<Vec<u8>> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| FocusError::invalid("revision", error.to_string()))?;
        if bytes.len() > MAX_REVISION_BYTES {
            return Err(FocusError::limit(
                "encoded revision bytes",
                MAX_REVISION_BYTES,
            ));
        }
        Ok(bytes)
    }

    /// The inclusions and exclusions of the embedded selection, as the codec reads them,
    /// with their labels.
    ///
    /// # Errors
    ///
    /// The codec's refusal: an unsupported version or an undecodable document.
    pub fn members(
        &self,
        codec: &dyn SelectionCodec,
    ) -> Result<(Vec<Member>, Vec<Member>), UnavailableReason> {
        let mut read = codec.decode(self.selection.get().as_bytes())?;
        for (members, labels) in [
            (&mut read.include, &self.labels.include),
            (&mut read.exclude, &self.labels.exclude),
        ] {
            for (member, label) in members.iter_mut().zip(labels) {
                label.clone_into(&mut member.label);
            }
        }
        Ok((read.include, read.exclude))
    }

    /// Whether this revision already holds `content` (labels, which only display, aside).
    #[must_use]
    pub fn holds(&self, codec: &dyn SelectionCodec, content: &FocusContent) -> bool {
        let Ok((include, exclude)) = self.members(codec) else {
            return false;
        };
        // The same members whatever order either side lists them in.
        let same = |stored: &[Member], wanted: &[Member]| {
            let mut stored: Vec<_> = stored.iter().map(Member::identity).collect();
            let mut wanted: Vec<_> = wanted.iter().map(Member::identity).collect();
            stored.sort_unstable();
            wanted.sort_unstable();
            stored == wanted
        };
        self.team == content.team
            && self.name == content.name
            && self.description == content.description
            && same(&include, &content.include)
            && same(&exclude, &content.exclude)
    }
}

/// Whether a definition accepts changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionState {
    /// The definition is live and holds its name.
    Active,
    /// The definition is final: it released its name and refuses changes.
    Archived,
}

/// The `meta` field of a definition record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// The definition.
    pub focus_id: FocusId,
    /// The team.
    pub team: String,
    /// The current name.
    pub name: String,
    /// Active or archived.
    pub state: DefinitionState,
    /// The latest revision number.
    pub latest_revision: u64,
    /// When the definition was archived, Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at_ms: Option<i64>,
    /// Members this build does not define, kept unchanged when it rewrites the field.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What the owner's session looked like when it last wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerBinding {
    /// The owner's machine.
    pub machine: String,
    /// The owner's native harness session ID.
    pub native_session_id: String,
    /// The owner's process, when the writer measured it (a transfer target's is not known
    /// until it next writes).
    pub process_id: Option<u32>,
    /// The owner's address when it last wrote; a display label, never identity.
    pub address: String,
    /// When the binding was last verified, in Unix milliseconds.
    pub verified_at: i64,
    /// Members this build does not define.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl OwnerBinding {
    /// A binding for session `session` on `machine`.
    #[must_use]
    pub fn new(
        machine: &MachineName,
        session: &SessionId,
        process_id: Option<u32>,
        address: &str,
        verified_at: i64,
    ) -> Self {
        Self {
            machine: machine.to_string(),
            native_session_id: session.to_string(),
            process_id,
            address: address.to_owned(),
            verified_at,
            extra: Map::new(),
        }
    }

    /// The identity `<machine>/<native session ID>`: the only thing that identifies an owner.
    #[must_use]
    pub fn owner_id(&self) -> String {
        format!("{}/{}", self.machine, self.native_session_id)
    }
}

/// How the owner record last changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// The definition was created.
    Create,
    /// A caller took over an absent or replaced owner.
    Claim,
    /// A caller took over a live or unknown owner with a stated reason.
    ForcedClaim,
    /// The owner handed the definition to a verified live session.
    Transfer,
}

impl ChangeKind {
    /// The kind as stored and printed.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Claim => "claim",
            Self::ForcedClaim => "forced_claim",
            Self::Transfer => "transfer",
        }
    }
}

/// The last change of the owner record only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerChange {
    /// What happened.
    pub kind: ChangeKind,
    /// The owner it replaced, absent for a creation.
    pub previous_owner_id: Option<String>,
    /// The reason a forced claim records.
    pub reason: Option<String>,
    /// When, in Unix milliseconds.
    pub at: i64,
    /// Members this build does not define.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The `owner` field of a definition record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    /// `<machine>/<native session ID>`.
    pub owner_id: String,
    /// Starts at 1 and grows by exactly 1 on every claim or transfer.
    pub epoch: u64,
    /// The owner's session when it last wrote.
    pub binding: OwnerBinding,
    /// The last change of the owner.
    pub change: OwnerChange,
    /// Members this build does not define.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One adopting session incarnation: `<machine>/<native session ID>/<process ID>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incarnation {
    /// The machine.
    pub machine: MachineName,
    /// The native harness session.
    pub session: SessionId,
    /// The process.
    pub process_id: u32,
}

impl Incarnation {
    /// The session of this incarnation as an owner ID, `<machine>/<session>`.
    #[must_use]
    pub fn session_key(&self) -> String {
        format!("{}/{}", self.machine, self.session)
    }
}

impl fmt::Display for Incarnation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.machine, self.session, self.process_id)
    }
}

impl FromStr for Incarnation {
    type Err = FocusError;

    fn from_str(text: &str) -> FocusResult<Self> {
        let bad = || {
            FocusError::invalid(
                "incarnation",
                format!("{text:?} is not `<machine>/<session>/<process ID>`"),
            )
        };
        let (machine, rest) = text.split_once('/').ok_or_else(bad)?;
        let (session, process) = rest.rsplit_once('/').ok_or_else(bad)?;
        Ok(Self {
            machine: machine.parse().map_err(|_| bad())?,
            session: session.parse().map_err(|_| bad())?,
            process_id: process.parse().map_err(|_| bad())?,
        })
    }
}

/// What a `pin put` records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewPin {
    /// The adopting incarnation.
    pub incarnation: Incarnation,
    /// The revision it adopted.
    pub adopted_revision: u64,
    /// When, in Unix milliseconds.
    pub pinned_at: i64,
}

/// The stored value of a pin, at most 512 bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinValue {
    /// `<machine>/<native session ID>/<process ID>`.
    pub incarnation: String,
    /// The revision it adopted.
    pub adopted_revision: u64,
    /// When, in Unix milliseconds.
    pub pinned_at: i64,
    /// Members this build does not define.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One pin as stored: its field, its exact stored bytes and, when it decodes, its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinEntry {
    /// The incarnation, from the field name `pin:<incarnation>`.
    pub incarnation: String,
    /// The stored bytes: what a compare-and-delete must still find.
    pub raw: Vec<u8>,
    /// The decoded value; `None` when the bytes do not decode, which keeps the pin.
    pub value: Option<PinValue>,
}

/// A definition's current state: what `list` shows for each.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// The `meta` field.
    pub meta: Meta,
    /// The `owner` field.
    pub owner: Owner,
    /// The latest revision.
    pub latest: Revision,
}

/// One definition as read: its summary, the revisions it keeps and its pins.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    /// Meta, owner and latest revision.
    pub summary: Summary,
    /// The numbers of the revisions kept, ascending.
    pub kept_revisions: Vec<u64>,
    /// The pins, by incarnation.
    pub pins: Vec<PinEntry>,
    pub(crate) meta_raw: Vec<u8>,
    pub(crate) owner_raw: Vec<u8>,
}

/// One entry of a listing: a usable definition, or one that cannot be used and why.
#[derive(Clone, Debug, PartialEq)]
pub enum ListEntry {
    /// A definition that decodes.
    Available(Box<Summary>),
    /// A definition that cannot be used; other definitions are still listed.
    Unavailable {
        /// Its ID.
        focus: FocusId,
        /// Why.
        reason: UnavailableReason,
    },
}

/// Decodes one definition's summary from its stored fields.
pub(crate) fn decode_summary(
    focus: &FocusId,
    meta_raw: &[u8],
    owner_raw: &[u8],
    latest_raw: Option<&[u8]>,
    codec: &dyn SelectionCodec,
) -> Result<Summary, UnavailableReason> {
    let undecodable = |_| UnavailableReason::Undecodable;
    let meta: Meta = serde_json::from_slice(meta_raw).map_err(undecodable)?;
    let owner: Owner = serde_json::from_slice(owner_raw).map_err(undecodable)?;
    if meta.focus_id != *focus {
        return Err(UnavailableReason::Undecodable);
    }
    let latest: Revision =
        serde_json::from_slice(latest_raw.ok_or(UnavailableReason::Undecodable)?)
            .map_err(undecodable)?;
    if latest.focus_id != *focus || latest.revision != meta.latest_revision {
        return Err(UnavailableReason::Undecodable);
    }
    codec.decode(latest.selection.get().as_bytes())?;
    Ok(Summary {
        meta,
        owner,
        latest,
    })
}

/// Decodes one stored revision and checks its embedded selection.
pub(crate) fn decode_revision(
    focus: &FocusId,
    raw: &[u8],
    codec: &dyn SelectionCodec,
) -> Result<Revision, UnavailableReason> {
    let revision: Revision =
        serde_json::from_slice(raw).map_err(|_| UnavailableReason::Undecodable)?;
    if revision.focus_id != *focus {
        return Err(UnavailableReason::Undecodable);
    }
    codec.decode(revision.selection.get().as_bytes())?;
    Ok(revision)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures")]

    use ix_trace_rs::trace;
    use serde_json::json;

    use super::*;

    fn content() -> FocusContent {
        FocusContent {
            team: "SWM".into(),
            name: "Plan".into(),
            description: String::new(),
            include: vec![Member::new(MemberKind::Project, "p1", "Project")],
            exclude: Vec::new(),
        }
    }

    #[trace("FR-061-AC-1")]
    #[test]
    fn a_generated_id_is_128_random_bits_in_lowercase_hex_and_round_trips() {
        let first = FocusId::generate().unwrap();
        let second = FocusId::generate().unwrap();
        assert_ne!(first, second);
        for id in [&first, &second] {
            assert_eq!(id.as_str().len(), "foc-".len() + 32);
            assert!(FocusId::is_well_formed(id.as_str()));
            assert_eq!(id.as_str().parse::<FocusId>().unwrap(), *id);
            let json = serde_json::to_string(id).unwrap();
            assert_eq!(serde_json::from_str::<FocusId>(&json).unwrap(), *id);
        }
    }

    #[trace("FR-061-AC-5")]
    #[test]
    fn only_a_focus_id_is_a_focus_id() {
        for bad in [
            "",
            "foc-",
            "foc-0123456789abcdef0123456789abcde",
            "foc-0123456789abcdef0123456789abcdef0",
            "foc-0123456789ABCDEF0123456789abcdef",
            "foc_0123456789abcdef0123456789abcdef",
            "bar-0123456789abcdef0123456789abcdef",
            "Release",
        ] {
            assert!(!FocusId::is_well_formed(bad), "{bad:?}");
            assert!(bad.parse::<FocusId>().is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<FocusId>("\"Release\"").is_err());
    }

    #[trace("FR-061-AC-12")]
    #[test]
    fn truncation_never_splits_a_utf8_sequence() {
        assert_eq!(cut_utf8("abc", 3), "abc");
        assert_eq!(cut_utf8("abc", 2), "ab");
        assert_eq!(cut_utf8("abc", 0), "");
        // "é" is two bytes: a cut inside it backs off to the character before.
        assert_eq!(cut_utf8("aé", 2), "a");
        assert_eq!(cut_utf8("aé", 3), "aé");
        assert_eq!(cut_utf8("ééé", 5), "éé");
        assert_eq!(cut_utf8("日本語", 4), "日");
        assert_eq!(cut_utf8("日本語", 0), "");
    }

    #[trace("FR-061-AC-12")]
    #[test]
    fn a_label_is_cut_to_64_bytes_on_a_character_boundary() {
        let mut wanted = content();
        wanted.include = vec![Member::new(MemberKind::Project, "p1", "é".repeat(40))];
        let normalized = wanted.normalize().unwrap();
        assert_eq!(normalized.include[0].label.len(), 64);
        assert!(normalized.include[0].label.chars().all(|c| c == 'é'));
        let mut odd = content();
        odd.include = vec![Member::new(
            MemberKind::Project,
            "p1",
            format!("x{}", "é".repeat(40)),
        )];
        let normalized = odd.normalize().unwrap();
        assert_eq!(
            normalized.include[0].label.len(),
            63,
            "a half character is dropped"
        );
    }

    #[trace("FR-061-AC-1")]
    #[test]
    fn members_are_deduplicated_and_ordered_canonically() {
        let mut wanted = content();
        wanted.include = vec![
            Member::new(MemberKind::Issue, "t2", "b"),
            Member::new(MemberKind::Project, "p2", "x"),
            Member::new(MemberKind::Project, "p1", "first"),
            Member::new(MemberKind::Project, "p1", "again"),
            Member::new(MemberKind::Initiative, "i1", "i"),
        ];
        let normalized = wanted.normalize().unwrap();
        let ids: Vec<&str> = normalized.include.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), 4, "the duplicate is dropped");
        assert_eq!(normalized.include[0].kind, MemberKind::Initiative);
        assert_eq!(
            normalized
                .include
                .iter()
                .find(|m| m.id == "p1")
                .map(|m| m.label.as_str()),
            Some("first"),
            "the first label of a duplicate is kept"
        );
    }

    #[trace("FR-061-AC-1")]
    #[test]
    fn a_name_is_sanitized_to_one_line_and_a_description_keeps_its_line_breaks() {
        let mut wanted = content();
        wanted.name = "Plan\none\ttwo\u{7}".into();
        wanted.description = "first\nsecond\u{7}".into();
        let normalized = wanted.normalize().unwrap();
        assert!(
            !normalized.name.contains(['\n', '\t', '\u{7}']),
            "{:?}",
            normalized.name
        );
        assert!(normalized.name.starts_with("Plan"));
        assert!(normalized.description.contains('\n'), "line breaks stay");
        assert!(
            !normalized.description.contains('\u{7}'),
            "unsafe characters go"
        );
        let mut blank = content();
        // Sanitizing replaces a line break with a visible character, so a name that is
        // blank is empty or spaces only.
        for name in ["", "   "] {
            blank.name = name.into();
            assert!(blank.clone().normalize().is_err(), "{name:?} is blank");
        }
    }

    #[trace("FR-061-AC-12")]
    #[test]
    fn the_bound_applies_to_the_stored_text_after_sanitizing() {
        let mut wanted = content();
        // 43 bytes of U+0007 become 43 three-byte replacement characters: 129 bytes.
        wanted.name = "\u{7}".repeat(43);
        let error = wanted.normalize().unwrap_err();
        assert_eq!(error.code().as_str(), "limit", "{error}");
    }

    #[trace("FR-061-AC-6")]
    #[test]
    fn an_owner_binding_keeps_the_members_it_does_not_define() {
        let stored = r#"{"machine":"terra","native_session_id":"s","process_id":7,
            "address":"terra:QSL:a","verified_at":3,"from_the_future":[1,2]}"#;
        let binding: OwnerBinding = serde_json::from_str(stored).unwrap();
        assert_eq!(binding.owner_id(), "terra/s");
        let again = serde_json::to_value(&binding).unwrap();
        assert_eq!(again["from_the_future"], json!([1, 2]));
        assert_eq!(again["process_id"], 7);
    }

    #[trace("FR-061-AC-15")]
    #[test]
    fn an_incarnation_names_a_machine_session_and_process() {
        let incarnation: Incarnation = "terra/sess-a/4242".parse().unwrap();
        assert_eq!(incarnation.process_id, 4242);
        assert_eq!(incarnation.session_key(), "terra/sess-a");
        assert_eq!(incarnation.to_string(), "terra/sess-a/4242");
        for bad in [
            "",
            "terra",
            "terra/sess-a",
            "terra/sess-a/x",
            "terra/sess-a/-1",
        ] {
            assert!(bad.parse::<Incarnation>().is_err(), "{bad:?}");
        }
    }
}
