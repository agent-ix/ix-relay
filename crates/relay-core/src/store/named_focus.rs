// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! Named focus definitions in Redis (FR-061).
//!
//! One hash per definition holds `meta`, `owner`, `rev:<n>` per kept revision and
//! `pin:<incarnation>` per pin. Two small indexes (definitions by ID, names by team and folded
//! name) are written in the same atomic operation as the record they describe. Nothing here
//! has a time-to-live. Every restricted write is planned from one read and committed by one
//! script that compares again before it writes (see [`crate::named_focus`]).

use std::collections::BTreeMap;

use redis::Commands;
use serde::{Deserialize, Serialize};
use serde_json::Map;

use super::{Relay, named_focus_scripts as scripts};
use crate::{
    Error,
    named_focus::{
        Applied, Caller, ChangeKind, ClaimRequest, Committed, CreateOutcome, Definition,
        DefinitionState, FocusContent, FocusError, FocusId, FocusResult, Incarnation, ListEntry,
        LivenessEvidence, Meta, NewPin, Owner, OwnerBinding, OwnerChange, PinEntry, PinValue,
        PlanKind, Planned, Resolved, ResolvedName, Revision, SelectionCheck, UnavailableReason,
        UpdateOutcome, WritePlan, contended,
        limits::{
            ARCHIVE_RETENTION_MS, KEPT_REVISIONS, MAX_ACTIVE_DEFINITIONS, MAX_DEFINITIONS,
            MAX_FIELD_BYTES, MAX_INDEX_ENTRIES, MAX_PIN_BYTES, MAX_PINS, MAX_REASON_BYTES,
            MAX_WRITE_ATTEMPTS, REMOVAL_EXAMINED,
        },
        model::{decode_revision, decode_summary},
    },
    text,
};

/// One entry of the definitions index. The names index maps `name_key` back to the ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct IndexEntry {
    team: String,
    name: String,
    name_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived_at_ms: Option<i64>,
}

impl IndexEntry {
    fn of(meta: &Meta) -> Self {
        Self {
            team: meta.team.clone(),
            name: meta.name.clone(),
            name_key: name_key(&meta.team, &meta.name),
            archived_at_ms: meta.archived_at_ms,
        }
    }
}

/// The key of a name in the names index: the team and the case-folded name.
fn name_key(team: &str, name: &str) -> String {
    format!("{team}\u{1f}{}", name.to_lowercase())
}

/// What one read of a definition's record found.
enum RawRead {
    Missing,
    TooLarge,
    Fields(BTreeMap<String, Vec<u8>>),
}

fn status(reply: &[Vec<u8>]) -> &str {
    reply
        .first()
        .and_then(|status| std::str::from_utf8(status).ok())
        .unwrap_or_default()
}

fn unexpected(script: &'static str, reply: &str) -> FocusError {
    FocusError::Store(Error::UnexpectedScriptReply {
        script,
        reply: reply.to_owned(),
    })
}

fn bytes<T: Serialize>(value: &T, field: &'static str) -> FocusResult<Vec<u8>> {
    let encoded =
        serde_json::to_vec(value).map_err(|error| FocusError::invalid(field, error.to_string()))?;
    if encoded.len() > MAX_FIELD_BYTES {
        return Err(FocusError::limit("bytes per stored field", MAX_FIELD_BYTES));
    }
    Ok(encoded)
}

fn number(value: u64) -> Vec<u8> {
    value.to_string().into_bytes()
}

fn unavailable(focus: &FocusId, reason: UnavailableReason) -> FocusError {
    FocusError::UnavailableDefinition {
        focus: focus.clone(),
        reason,
    }
}

/// The refusal a script answered with, built from the state its comparison saw.
fn refusal(
    plan_focus: &FocusId,
    reply: &[Vec<u8>],
    moved: Option<&(String, String)>,
) -> FocusError {
    let focus = plan_focus.clone();
    match status(reply) {
        "not_found" => FocusError::NotFound {
            focus: focus.to_string(),
        },
        "undecodable" => unavailable(&focus, UnavailableReason::Undecodable),
        "archived" => FocusError::Archived { focus },
        other => {
            let state = reply.get(1).zip(reply.get(2)).and_then(|(meta, owner)| {
                Some((
                    serde_json::from_slice::<Meta>(meta).ok()?,
                    serde_json::from_slice::<Owner>(owner).ok()?,
                ))
            });
            let Some((meta, owner)) = state else {
                return unavailable(&focus, UnavailableReason::Undecodable);
            };
            match other {
                "not_owner" => FocusError::NotOwner {
                    detail: format!(
                        "the verified caller is not the owner of {focus}; the owner is {}",
                        owner.owner_id
                    ),
                    focus,
                    owner_id: owner.owner_id,
                    epoch: owner.epoch,
                },
                "owner_epoch_conflict" => FocusError::OwnerEpochConflict {
                    focus,
                    owner_id: owner.owner_id,
                    epoch: owner.epoch,
                },
                "revision_conflict" => FocusError::RevisionConflict {
                    focus,
                    latest: meta.latest_revision,
                },
                "name_moved" => {
                    let (team, name) = moved.cloned().unwrap_or((meta.team, meta.name));
                    FocusError::NameTaken {
                        team,
                        name,
                        holder: None,
                    }
                }
                _ => unexpected("focus-write", other),
            }
        }
    }
}

fn decode_definition(
    focus: &FocusId,
    fields: &BTreeMap<String, Vec<u8>>,
    check: &dyn SelectionCheck,
) -> Result<Definition, UnavailableReason> {
    let meta_raw = fields.get("meta").ok_or(UnavailableReason::Undecodable)?;
    let owner_raw = fields.get("owner").ok_or(UnavailableReason::Undecodable)?;
    let meta: Meta =
        serde_json::from_slice(meta_raw).map_err(|_| UnavailableReason::Undecodable)?;
    let latest = fields
        .get(&format!("rev:{}", meta.latest_revision))
        .map(Vec::as_slice);
    let summary = decode_summary(focus, meta_raw, owner_raw, latest, check)?;
    let mut kept_revisions: Vec<u64> = fields
        .keys()
        .filter_map(|field| field.strip_prefix("rev:")?.parse().ok())
        .collect();
    kept_revisions.sort_unstable();
    let pins = fields
        .iter()
        .filter_map(|(field, raw)| {
            Some(PinEntry {
                incarnation: field.strip_prefix("pin:")?.to_owned(),
                raw: raw.clone(),
                value: serde_json::from_slice(raw).ok(),
            })
        })
        .collect();
    Ok(Definition {
        summary,
        kept_revisions,
        pins,
        meta_raw: meta_raw.clone(),
        owner_raw: owner_raw.clone(),
    })
}

/// The owner record after the same owner wrote again: the stored binding is refreshed
/// (`process_id`, `address`, `verified_at`) and nothing else changes.
fn refreshed_owner(owner: &Owner, caller: &OwnerBinding) -> Owner {
    let mut next = owner.clone();
    next.binding = OwnerBinding {
        extra: owner.binding.extra.clone(),
        ..caller.clone()
    };
    next
}

/// The owner record after `binding` took over from `previous`.
fn successor_owner(
    previous: &Owner,
    binding: &OwnerBinding,
    kind: ChangeKind,
    reason: Option<String>,
    now_ms: i64,
) -> FocusResult<Owner> {
    let epoch = previous.epoch.checked_add(1).ok_or(FocusError::Limit {
        bound: "owner epoch",
        max: u64::MAX,
    })?;
    Ok(Owner {
        owner_id: binding.owner_id(),
        epoch,
        binding: binding.clone(),
        change: OwnerChange {
            kind,
            previous_owner_id: Some(previous.owner_id.clone()),
            reason,
            at_ms: now_ms,
            extra: Map::new(),
        },
        extra: previous.extra.clone(),
    })
}

/// Refuses a caller that is not the stored owner at the epoch it verified.
fn owner_gate(def: &Definition, caller: &Caller<'_>) -> FocusResult<()> {
    let focus = def.summary.meta.focus_id.clone();
    let owner = &def.summary.owner;
    if owner.owner_id != caller.binding.owner_id() {
        return Err(FocusError::NotOwner {
            detail: format!(
                "the verified caller is not the owner of {focus}; the owner is {}",
                owner.owner_id
            ),
            focus,
            owner_id: owner.owner_id.clone(),
            epoch: owner.epoch,
        });
    }
    if owner.epoch != caller.expected_epoch {
        return Err(FocusError::OwnerEpochConflict {
            focus,
            owner_id: owner.owner_id.clone(),
            epoch: owner.epoch,
        });
    }
    Ok(())
}

fn validate_reason(reason: &str) -> FocusResult<String> {
    if reason.trim().is_empty() {
        return Err(FocusError::invalid(
            "reason",
            "a forced claim needs a reason",
        ));
    }
    if reason != text::sanitise_line(reason) {
        return Err(FocusError::invalid(
            "reason",
            "must be one line of plain text without control characters",
        ));
    }
    if reason.len() > MAX_REASON_BYTES {
        return Err(FocusError::limit("reason bytes", MAX_REASON_BYTES));
    }
    Ok(reason.to_owned())
}

impl Relay {
    /// One read of the registry as session liveness evidence (FR-061 "Session liveness
    /// evidence"). A read that fails yields evidence under which no session is ever absent:
    /// an unknown pin or owner is kept, never shown as live and never counted as absent.
    pub fn focus_evidence(&mut self) -> LivenessEvidence {
        self.agents().map_or_else(
            |_| LivenessEvidence::unreadable(),
            |list| LivenessEvidence::from_registry(&list),
        )
    }

    fn focus_script(
        &mut self,
        source: &str,
        keys: &[String],
        args: &[Vec<u8>],
    ) -> crate::Result<Vec<Vec<u8>>> {
        let script = redis::Script::new(source);
        let mut call = script.prepare_invoke();
        for key in keys {
            call.key(key.as_str());
        }
        for arg in args {
            call.arg(arg.as_slice());
        }
        call.invoke(&mut self.conn).map_err(Error::from)
    }

    /// Reads the definitions index: every definition's ID and, when it decodes, its entry.
    fn focus_index(&mut self) -> FocusResult<Vec<(FocusId, Option<IndexEntry>)>> {
        let key = self.keys.focus_index_key();
        let len: usize = self.conn.hlen(&key).map_err(Error::from)?;
        if len > MAX_INDEX_ENTRIES {
            return Err(unexpected("focus-index", &format!("{len} entries")));
        }
        let raw: Vec<(Vec<u8>, Vec<u8>)> = self.conn.hgetall(&key).map_err(Error::from)?;
        Ok(raw
            .into_iter()
            .filter_map(|(id, entry)| {
                let id = std::str::from_utf8(&id).ok()?.parse().ok()?;
                Some((id, serde_json::from_slice(&entry).ok()))
            })
            .collect())
    }

    fn focus_read_raw(&mut self, id: &FocusId) -> FocusResult<RawRead> {
        let key = self.keys.focus_key(id);
        let reply = self.focus_script(&scripts::plain(scripts::READ_ONE), &[key], &[])?;
        match status(&reply) {
            "missing" => Ok(RawRead::Missing),
            "toolarge" => Ok(RawRead::TooLarge),
            "ok" => Ok(RawRead::Fields(
                reply
                    .get(1..)
                    .unwrap_or_default()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|[field, value]| {
                        (String::from_utf8_lossy(field).into_owned(), value.clone())
                    })
                    .collect(),
            )),
            other => Err(unexpected("focus-read", other)),
        }
    }

    /// Resolves a reference that is a focus ID or the exact current name of a non-archived
    /// definition. An ID is not checked against the store here; the read that follows is.
    ///
    /// # Errors
    ///
    /// [`FocusError::NotFound`] when no non-archived definition has the name, and
    /// [`FocusError::InvalidArgument`] when the name belongs to definitions of several teams.
    pub fn focus_resolve(&mut self, reference: &str) -> FocusResult<Resolved> {
        if FocusId::is_well_formed(reference) {
            return Ok(Resolved::by_id(reference.parse()?));
        }
        let mut matches: Vec<(FocusId, IndexEntry)> = self
            .focus_index()?
            .into_iter()
            .filter_map(|(id, entry)| Some((id, entry?)))
            .filter(|(_, entry)| entry.name == reference && entry.archived_at_ms.is_none())
            .collect();
        match matches.len() {
            0 => Err(FocusError::NotFound {
                focus: reference.to_owned(),
            }),
            1 => {
                let (id, entry) = matches.remove(0);
                Ok(Resolved {
                    id,
                    by_name: Some(ResolvedName {
                        team: entry.team,
                        name: entry.name,
                        key: entry.name_key,
                    }),
                })
            }
            _ => Err(FocusError::invalid(
                "focus",
                format!(
                    "the name {reference:?} belongs to definitions of several teams; use the focus ID"
                ),
            )),
        }
    }

    /// Reads one definition: summary, kept revisions and pins.
    ///
    /// # Errors
    ///
    /// [`FocusError::NotFound`] for an ID that is not stored and
    /// [`FocusError::UnavailableDefinition`] for a record that cannot be used, which a read
    /// never overwrites.
    pub fn focus_get(
        &mut self,
        check: &dyn SelectionCheck,
        id: &FocusId,
    ) -> FocusResult<Definition> {
        match self.focus_read_raw(id)? {
            RawRead::Missing => Err(FocusError::NotFound {
                focus: id.to_string(),
            }),
            RawRead::TooLarge => Err(unavailable(id, UnavailableReason::Undecodable)),
            RawRead::Fields(fields) => {
                decode_definition(id, &fields, check).map_err(|reason| unavailable(id, reason))
            }
        }
    }

    /// Reads one kept revision.
    ///
    /// # Errors
    ///
    /// As [`Relay::focus_get`], and [`FocusError::NotFound`] when revision `number` is not
    /// kept (the eight newest are).
    pub fn focus_revision(
        &mut self,
        check: &dyn SelectionCheck,
        id: &FocusId,
        number: u64,
    ) -> FocusResult<Revision> {
        let definition = self.focus_get(check, id)?;
        if !definition.kept_revisions.contains(&number) {
            return Err(FocusError::NotFound {
                focus: format!(
                    "revision {number} of {id} (kept: {:?})",
                    definition.kept_revisions
                ),
            });
        }
        let key = self.keys.focus_key(id);
        let length: usize = redis::cmd("HSTRLEN")
            .arg(&key)
            .arg(format!("rev:{number}"))
            .query(&mut self.conn)
            .map_err(Error::from)?;
        if length > MAX_FIELD_BYTES {
            return Err(unavailable(id, UnavailableReason::Undecodable));
        }
        let raw: Option<Vec<u8>> = self
            .conn
            .hget(&key, format!("rev:{number}"))
            .map_err(Error::from)?;
        let raw = raw.ok_or_else(|| FocusError::NotFound {
            focus: format!("revision {number} of {id}"),
        })?;
        decode_revision(id, &raw, check).map_err(|reason| unavailable(id, reason))
    }

    /// Every definition, in one bounded read: the usable ones sorted by team and name, and the
    /// ones that cannot be used, by ID, each with its reason. One unusable definition never
    /// hides the others.
    ///
    /// # Errors
    ///
    /// Only Redis failing, or an index larger than any the relay writes.
    pub fn focus_list(&mut self, check: &dyn SelectionCheck) -> FocusResult<Vec<ListEntry>> {
        let reply = self.focus_script(
            &scripts::plain(scripts::LIST),
            &[self.keys.focus_index_key()],
            &[self.keys.focus_stem().into_bytes()],
        )?;
        match status(&reply) {
            "ok" => {}
            other => return Err(unexpected("focus-list", other)),
        }
        let mut usable = Vec::new();
        let mut unusable = Vec::new();
        for [id, meta, owner, revision] in reply.get(1..).unwrap_or_default().as_chunks::<4>().0 {
            let Some(id) = std::str::from_utf8(id)
                .ok()
                .and_then(|id| id.parse::<FocusId>().ok())
            else {
                continue;
            };
            let latest = (!revision.is_empty()).then_some(revision.as_slice());
            match decode_summary(&id, meta, owner, latest, check) {
                Ok(summary) => usable.push(summary),
                Err(reason) => unusable.push((id, reason)),
            }
        }
        usable.sort_by(|a, b| {
            (&a.meta.team, a.meta.name.to_lowercase(), &a.meta.focus_id).cmp(&(
                &b.meta.team,
                b.meta.name.to_lowercase(),
                &b.meta.focus_id,
            ))
        });
        unusable.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(usable
            .into_iter()
            .map(|summary| ListEntry::Available(Box::new(summary)))
            .chain(
                unusable
                    .into_iter()
                    .map(|(focus, reason)| ListEntry::Unavailable { focus, reason }),
            )
            .collect())
    }

    /// Creates a definition: revision 1 and the caller as owner at epoch 1, in one atomic
    /// operation that also writes both indexes.
    ///
    /// When the 256-definition limit would refuse it, at most eight archived definitions are
    /// examined, oldest first, after reclaiming their dead pins; the first archived for at least
    /// 90 days that holds no pin is removed to make room. Nothing else is ever evicted.
    ///
    /// # Errors
    ///
    /// [`FocusError::NameTaken`], [`FocusError::Limit`] naming the bound, and the refusals of
    /// [`FocusContent::normalize`]. A refusal writes nothing.
    pub fn focus_create(
        &mut self,
        check: &dyn SelectionCheck,
        content: FocusContent,
        owner: &OwnerBinding,
        now_ms: i64,
        evidence: &LivenessEvidence,
    ) -> FocusResult<CreateOutcome> {
        let content = content.normalize()?;
        let candidates = self.focus_removal_candidates(now_ms, evidence)?;
        for _ in 0..MAX_WRITE_ATTEMPTS {
            let id = FocusId::generate()?;
            let revision = Revision::build(&id, 1, &content, check.schema_version(), 1, now_ms);
            check
                .check(&bytes(&revision.selection, "selection")?)
                .map_err(|reason| unavailable(&id, reason))?;
            let meta = Meta {
                focus_id: id.clone(),
                team: content.team.clone(),
                name: content.name.clone(),
                state: DefinitionState::Active,
                latest_revision: 1,
                archived_at_ms: None,
                extra: Map::new(),
            };
            let record = Owner {
                owner_id: owner.owner_id(),
                epoch: 1,
                binding: owner.clone(),
                change: OwnerChange {
                    kind: ChangeKind::Create,
                    previous_owner_id: None,
                    reason: None,
                    at_ms: now_ms,
                    extra: Map::new(),
                },
                extra: Map::new(),
            };
            let index = IndexEntry::of(&meta);
            let mut args = vec![
                index.name_key.clone().into_bytes(),
                bytes(&meta, "meta")?,
                bytes(&record, "owner")?,
                revision.encode()?,
                bytes(&index, "index")?,
                id.to_string().into_bytes(),
                number(MAX_ACTIVE_DEFINITIONS as u64),
                number(MAX_DEFINITIONS as u64),
                now_ms.to_string().into_bytes(),
                ARCHIVE_RETENTION_MS.to_string().into_bytes(),
                self.keys.focus_stem().into_bytes(),
            ];
            args.extend(candidates.iter().map(|c| c.to_string().into_bytes()));
            let keys = [
                self.keys.focus_key(&id),
                self.keys.focus_index_key(),
                self.keys.focus_names_key(),
            ];
            let reply = self.focus_script(&scripts::plain(scripts::CREATE), &keys, &args)?;
            match status(&reply) {
                "ok" => {
                    let removed = reply
                        .get(1)
                        .and_then(|id| std::str::from_utf8(id).ok())
                        .filter(|id| !id.is_empty())
                        .and_then(|id| id.parse().ok());
                    return Ok(CreateOutcome {
                        meta,
                        owner: record,
                        revision,
                        removed,
                    });
                }
                "exists" => {}
                "name_taken" => {
                    return Err(FocusError::NameTaken {
                        team: content.team,
                        name: content.name,
                        holder: reply
                            .get(1)
                            .and_then(|id| std::str::from_utf8(id).ok())
                            .and_then(|id| id.parse().ok()),
                    });
                }
                "limit_active" => {
                    return Err(FocusError::limit(
                        "non-archived definitions",
                        MAX_ACTIVE_DEFINITIONS,
                    ));
                }
                "limit_total" => {
                    return Err(FocusError::limit(
                        "definitions including archived",
                        MAX_DEFINITIONS,
                    ));
                }
                "undecodable" => {
                    return Err(unexpected(
                        "focus-create",
                        "the definitions index is damaged",
                    ));
                }
                other => return Err(unexpected("focus-create", other)),
            }
        }
        Err(unexpected(
            "focus-create",
            "a generated focus ID already exists",
        ))
    }

    /// The archived definitions `create` may remove, oldest archived first: only when the
    /// definition limit is reached, at most eight examined, each after reclaiming its dead
    /// pins, and only those archived for at least 90 days. The commit re-checks all of it.
    fn focus_removal_candidates(
        &mut self,
        now_ms: i64,
        evidence: &LivenessEvidence,
    ) -> FocusResult<Vec<FocusId>> {
        let entries = self.focus_index()?;
        if entries.len() < MAX_DEFINITIONS {
            return Ok(Vec::new());
        }
        let mut archived: Vec<(i64, FocusId)> = entries
            .into_iter()
            .filter_map(|(id, entry)| Some((entry?.archived_at_ms?, id)))
            .collect();
        archived.sort();
        let cutoff = now_ms.saturating_sub(ARCHIVE_RETENTION_MS);
        let mut candidates = Vec::new();
        for (archived_at, id) in archived.into_iter().take(REMOVAL_EXAMINED) {
            if archived_at > cutoff {
                break;
            }
            self.focus_reclaim_pins(&id, evidence)?;
            candidates.push(id);
        }
        Ok(candidates)
    }

    fn focus_keys(&self, id: &FocusId) -> Vec<String> {
        vec![
            self.keys.focus_key(id),
            self.keys.focus_index_key(),
            self.keys.focus_names_key(),
        ]
    }

    /// Plans the next revision of a definition from `content`, a complete replacement.
    ///
    /// Refuses what the read already refutes: an archived definition, a caller that is not the
    /// owner at the epoch it verified, a stale `expected_revision`, a changed team and revision
    /// overflow. Content equal to the current revision plans nothing.
    ///
    /// # Errors
    ///
    /// The typed refusals named above, and those of [`FocusContent::normalize`].
    pub fn focus_plan_update(
        &mut self,
        check: &dyn SelectionCheck,
        target: &Resolved,
        expected_revision: u64,
        content: FocusContent,
        caller: &Caller<'_>,
    ) -> FocusResult<Planned> {
        let content = content.normalize()?;
        let def = self.focus_get(check, &target.id)?;
        let meta = &def.summary.meta;
        if meta.state == DefinitionState::Archived {
            return Err(FocusError::Archived {
                focus: target.id.clone(),
            });
        }
        owner_gate(&def, caller)?;
        if meta.latest_revision != expected_revision {
            return Err(FocusError::RevisionConflict {
                focus: target.id.clone(),
                latest: meta.latest_revision,
            });
        }
        if content.team != meta.team {
            return Err(FocusError::invalid(
                "team",
                "a definition keeps its team; create a new definition for another team",
            ));
        }
        if def.summary.latest.holds(&content) {
            return Ok(Planned::Unchanged(Box::new(def.summary.latest)));
        }
        let next = meta
            .latest_revision
            .checked_add(1)
            .ok_or(FocusError::Limit {
                bound: "revision number",
                max: u64::MAX,
            })?;
        let revision = Revision::build(
            &target.id,
            next,
            &content,
            check.schema_version(),
            def.summary.owner.epoch,
            caller.now_ms,
        );
        check
            .check(&bytes(&revision.selection, "selection")?)
            .map_err(|reason| unavailable(&target.id, reason))?;
        let mut new_meta = meta.clone();
        new_meta.name = content.name.clone();
        new_meta.latest_revision = next;
        let owner = refreshed_owner(&def.summary.owner, caller.binding);
        let index = IndexEntry::of(&new_meta);
        let args = vec![
            number(expected_revision),
            caller.binding.owner_id().into_bytes(),
            number(caller.expected_epoch),
            def.meta_raw.clone(),
            def.owner_raw.clone(),
            bytes(&new_meta, "meta")?,
            bytes(&owner, "owner")?,
            revision.encode()?,
            number(next),
            index.name_key.clone().into_bytes(),
            bytes(&index, "index")?,
            target.name_key().into_bytes(),
            number(KEPT_REVISIONS),
            name_key(&meta.team, &meta.name).into_bytes(),
        ];
        Ok(Planned::Write(Box::new(WritePlan {
            kind: PlanKind::Update,
            focus: target.id.clone(),
            keys: self.focus_keys(&target.id),
            args,
            new_name: Some((content.team.clone(), content.name.clone())),
            resolved_name: target
                .by_name
                .as_ref()
                .map(|name| (name.team.clone(), name.name.clone())),
            applied: Applied::Revision(Box::new(revision)),
        })))
    }

    /// Plans archiving a definition: final, releases its name, writes no revision.
    ///
    /// # Errors
    ///
    /// [`FocusError::Archived`] when it already is, plus the owner and revision refusals of
    /// [`Relay::focus_plan_update`].
    pub fn focus_plan_archive(
        &mut self,
        check: &dyn SelectionCheck,
        target: &Resolved,
        expected_revision: u64,
        caller: &Caller<'_>,
    ) -> FocusResult<WritePlan> {
        let def = self.focus_get(check, &target.id)?;
        let meta = &def.summary.meta;
        if meta.state == DefinitionState::Archived {
            return Err(FocusError::Archived {
                focus: target.id.clone(),
            });
        }
        owner_gate(&def, caller)?;
        if meta.latest_revision != expected_revision {
            return Err(FocusError::RevisionConflict {
                focus: target.id.clone(),
                latest: meta.latest_revision,
            });
        }
        let mut new_meta = meta.clone();
        new_meta.state = DefinitionState::Archived;
        new_meta.archived_at_ms = Some(caller.now_ms);
        let owner = refreshed_owner(&def.summary.owner, caller.binding);
        let args = vec![
            number(expected_revision),
            caller.binding.owner_id().into_bytes(),
            number(caller.expected_epoch),
            def.meta_raw.clone(),
            def.owner_raw.clone(),
            bytes(&new_meta, "meta")?,
            bytes(&owner, "owner")?,
            bytes(&IndexEntry::of(&new_meta), "index")?,
            target.name_key().into_bytes(),
            name_key(&meta.team, &meta.name).into_bytes(),
        ];
        Ok(WritePlan {
            kind: PlanKind::Archive,
            focus: target.id.clone(),
            keys: self.focus_keys(&target.id),
            args,
            new_name: None,
            resolved_name: target
                .by_name
                .as_ref()
                .map(|name| (name.team.clone(), name.name.clone())),
            applied: Applied::Archived(Box::new(new_meta)),
        })
    }

    /// Plans a claim: the verified caller becomes owner, the epoch grows by 1.
    ///
    /// The claim carries the owner and epoch the caller *observed*; the caller's own ID is never
    /// compared with the stored one. Without `force_reason` it needs the owner's session to be
    /// absent or replaced by `evidence`; a live or unknown owner needs `--force --reason`.
    ///
    /// # Errors
    ///
    /// [`FocusError::OwnerEpochConflict`] when the stored owner is not the observed one at the
    /// observed epoch, [`FocusError::NotOwner`] for a live or unknown owner without a forced
    /// reason, and the refusals of an unusable definition.
    pub fn focus_plan_claim(
        &mut self,
        check: &dyn SelectionCheck,
        target: &Resolved,
        caller: &Caller<'_>,
        claim: &ClaimRequest<'_>,
        evidence: &LivenessEvidence,
    ) -> FocusResult<WritePlan> {
        let def = self.focus_get(check, &target.id)?;
        let owner = &def.summary.owner;
        if owner.owner_id != claim.observed_owner_id || owner.epoch != caller.expected_epoch {
            return Err(FocusError::OwnerEpochConflict {
                focus: target.id.clone(),
                owner_id: owner.owner_id.clone(),
                epoch: owner.epoch,
            });
        }
        let reason = claim.force_reason.map(validate_reason).transpose()?;
        if reason.is_none() {
            let state = evidence.owner_state(&owner.owner_id, &owner.binding.address);
            if !state.is_gone() {
                return Err(FocusError::NotOwner {
                    detail: format!(
                        "the owner of {} ({}) is {state}; a claim needs the owner to be absent or \
                         replaced, or `--force --reason <text>`",
                        target.id, owner.owner_id
                    ),
                    focus: target.id.clone(),
                    owner_id: owner.owner_id.clone(),
                    epoch: owner.epoch,
                });
            }
        }
        let kind = if reason.is_some() {
            ChangeKind::ForcedClaim
        } else {
            ChangeKind::Claim
        };
        let next = successor_owner(owner, caller.binding, kind, reason, caller.now_ms)?;
        let args = vec![
            claim.observed_owner_id.as_bytes().to_vec(),
            number(caller.expected_epoch),
            def.meta_raw.clone(),
            def.owner_raw.clone(),
            bytes(&next, "owner")?,
            target.name_key().into_bytes(),
        ];
        Ok(self.owner_plan(PlanKind::Claim, target, args, next))
    }

    /// Plans a transfer to a verified live session: only the owner record changes, and the
    /// epoch grows by 1. It does not activate the focus for the new owner.
    ///
    /// # Errors
    ///
    /// [`FocusError::Archived`], [`FocusError::NotOwner`], [`FocusError::OwnerEpochConflict`]
    /// and [`FocusError::InvalidArgument`] for a transfer to the current owner.
    pub fn focus_plan_transfer(
        &mut self,
        check: &dyn SelectionCheck,
        target: &Resolved,
        caller: &Caller<'_>,
        to: &OwnerBinding,
    ) -> FocusResult<WritePlan> {
        let def = self.focus_get(check, &target.id)?;
        if def.summary.meta.state == DefinitionState::Archived {
            return Err(FocusError::Archived {
                focus: target.id.clone(),
            });
        }
        owner_gate(&def, caller)?;
        if to.owner_id() == def.summary.owner.owner_id {
            return Err(FocusError::invalid(
                "to",
                "the target session already owns this definition",
            ));
        }
        let next = successor_owner(
            &def.summary.owner,
            to,
            ChangeKind::Transfer,
            None,
            caller.now_ms,
        )?;
        let args = vec![
            caller.binding.owner_id().into_bytes(),
            number(caller.expected_epoch),
            def.meta_raw.clone(),
            def.owner_raw.clone(),
            bytes(&next, "owner")?,
            target.name_key().into_bytes(),
        ];
        Ok(self.owner_plan(PlanKind::Transfer, target, args, next))
    }

    fn owner_plan(
        &self,
        kind: PlanKind,
        target: &Resolved,
        args: Vec<Vec<u8>>,
        next: Owner,
    ) -> WritePlan {
        WritePlan {
            kind,
            focus: target.id.clone(),
            keys: self.focus_keys(&target.id),
            args,
            new_name: None,
            resolved_name: target
                .by_name
                .as_ref()
                .map(|name| (name.team.clone(), name.name.clone())),
            applied: Applied::Owner(Box::new(next)),
        }
    }

    /// Applies a plan in one atomic server-side operation that re-reads the stored record and
    /// compares before it writes: the latest revision, the owner and its epoch, the state, and
    /// that `meta` and `owner` are still what the plan was built from.
    ///
    /// # Errors
    ///
    /// The refusal the comparison found, carrying the state it saw. Nothing is written then.
    pub fn focus_commit(&mut self, plan: &WritePlan) -> FocusResult<Committed> {
        let body = match plan.kind {
            PlanKind::Update => scripts::UPDATE,
            PlanKind::Archive => scripts::ARCHIVE,
            PlanKind::Claim => scripts::CLAIM,
            PlanKind::Transfer => scripts::TRANSFER,
        };
        let reply = self.focus_script(&scripts::guarded(body), &plan.keys, &plan.args)?;
        match status(&reply) {
            "ok" => Ok(Committed::Applied),
            "retry" => Ok(Committed::Retry),
            "name_taken" => {
                let (team, name) = plan.new_name.clone().unwrap_or_default();
                Err(FocusError::NameTaken {
                    team,
                    name,
                    holder: reply
                        .get(1)
                        .and_then(|id| std::str::from_utf8(id).ok())
                        .and_then(|id| id.parse().ok()),
                })
            }
            _ => Err(refusal(&plan.focus, &reply, plan.resolved_name.as_ref())),
        }
    }

    /// Writes the next revision: [`Relay::focus_plan_update`] and [`Relay::focus_commit`],
    /// planned again when a concurrent write changed an unrelated part of the record.
    ///
    /// Before the write, pins whose session is absent or replaced are reclaimed (never one
    /// whose session is live or unknown), outside the atomic operation.
    ///
    /// # Errors
    ///
    /// The refusals of the two steps; a stale expectation is [`FocusError::RevisionConflict`]
    /// carrying the latest revision.
    pub fn focus_update(
        &mut self,
        check: &dyn SelectionCheck,
        target: &Resolved,
        expected_revision: u64,
        content: &FocusContent,
        caller: &Caller<'_>,
        evidence: &LivenessEvidence,
    ) -> FocusResult<UpdateOutcome> {
        for _ in 0..MAX_WRITE_ATTEMPTS {
            match self.focus_plan_update(
                check,
                target,
                expected_revision,
                content.clone(),
                caller,
            )? {
                Planned::Unchanged(current) => return Ok(UpdateOutcome::Unchanged(current)),
                Planned::Write(plan) => {
                    self.focus_reclaim_pins(&target.id, evidence)?;
                    if self.focus_commit(&plan)? == Committed::Applied
                        && let Applied::Revision(written) = plan.applied
                    {
                        return Ok(UpdateOutcome::Written(written));
                    }
                }
            }
        }
        Err(contended(&target.id))
    }

    /// Archives a definition: plan, commit, and plan again after an unrelated concurrent write.
    ///
    /// # Errors
    ///
    /// As [`Relay::focus_plan_archive`] and [`Relay::focus_commit`].
    pub fn focus_archive(
        &mut self,
        check: &dyn SelectionCheck,
        target: &Resolved,
        expected_revision: u64,
        caller: &Caller<'_>,
    ) -> FocusResult<Meta> {
        for _ in 0..MAX_WRITE_ATTEMPTS {
            let plan = self.focus_plan_archive(check, target, expected_revision, caller)?;
            if self.focus_commit(&plan)? == Committed::Applied
                && let Applied::Archived(meta) = plan.applied
            {
                return Ok(*meta);
            }
        }
        Err(contended(&target.id))
    }

    /// Takes ownership: plan, commit, and plan again after an unrelated concurrent write.
    ///
    /// # Errors
    ///
    /// As [`Relay::focus_plan_claim`] and [`Relay::focus_commit`].
    pub fn focus_claim(
        &mut self,
        check: &dyn SelectionCheck,
        target: &Resolved,
        caller: &Caller<'_>,
        claim: &ClaimRequest<'_>,
        evidence: &LivenessEvidence,
    ) -> FocusResult<Owner> {
        for _ in 0..MAX_WRITE_ATTEMPTS {
            let plan = self.focus_plan_claim(check, target, caller, claim, evidence)?;
            if self.focus_commit(&plan)? == Committed::Applied
                && let Applied::Owner(owner) = plan.applied
            {
                return Ok(*owner);
            }
        }
        Err(contended(&target.id))
    }

    /// Hands ownership to `to`: plan, commit, and plan again after an unrelated concurrent write.
    ///
    /// # Errors
    ///
    /// As [`Relay::focus_plan_transfer`] and [`Relay::focus_commit`].
    pub fn focus_transfer(
        &mut self,
        check: &dyn SelectionCheck,
        target: &Resolved,
        caller: &Caller<'_>,
        to: &OwnerBinding,
    ) -> FocusResult<Owner> {
        for _ in 0..MAX_WRITE_ATTEMPTS {
            let plan = self.focus_plan_transfer(check, target, caller, to)?;
            if self.focus_commit(&plan)? == Committed::Applied
                && let Applied::Owner(owner) = plan.applied
            {
                return Ok(*owner);
            }
        }
        Err(contended(&target.id))
    }

    /// Records that one session incarnation adopted a definition: the only writer of a pin.
    ///
    /// Creates or replaces the incarnation's pin; refuses an archived definition and a 33rd
    /// incarnation. At the adopters limit it first reclaims the pins whose session is absent or
    /// replaced and retries once.
    ///
    /// # Errors
    ///
    /// [`FocusError::Archived`], and [`FocusError::Limit`] naming the adopters limit when no
    /// pin could be reclaimed.
    pub fn focus_pin_put(
        &mut self,
        check: &dyn SelectionCheck,
        id: &FocusId,
        pin: &NewPin,
        evidence: &LivenessEvidence,
    ) -> FocusResult<()> {
        self.focus_get(check, id)?;
        let adopters = || FocusError::limit("adopters per definition", MAX_PINS);
        if self.focus_pin_put_once(id, pin)? {
            return Ok(());
        }
        if self.focus_reclaim_pins(id, evidence)? == 0 || !self.focus_pin_put_once(id, pin)? {
            return Err(adopters());
        }
        Ok(())
    }

    /// One attempt to store a pin: `false` when the adopters limit refused a new incarnation.
    fn focus_pin_put_once(&mut self, id: &FocusId, pin: &NewPin) -> FocusResult<bool> {
        let value = PinValue {
            incarnation: pin.incarnation.to_string(),
            adopted_revision: pin.adopted_revision,
            pinned_at_ms: pin.pinned_at_ms,
            extra: Map::new(),
        };
        let encoded = serde_json::to_vec(&value)
            .map_err(|error| FocusError::invalid("pin", error.to_string()))?;
        if encoded.len() > MAX_PIN_BYTES {
            return Err(FocusError::limit("pin bytes", MAX_PIN_BYTES));
        }
        let args = [
            format!("pin:{}", pin.incarnation).into_bytes(),
            encoded,
            number(MAX_PINS as u64),
        ];
        let reply = self.focus_script(
            &scripts::guarded(scripts::PIN_PUT),
            &[self.keys.focus_key(id)],
            &args,
        )?;
        match status(&reply) {
            "ok" => Ok(true),
            "limit" => Ok(false),
            _ => Err(refusal(id, &reply, None)),
        }
    }

    /// Releases a pin only if its stored value still equals the value the caller read
    /// (compare-and-delete), so a concurrent re-pin is never lost. Returns whether it was
    /// removed.
    ///
    /// # Errors
    ///
    /// Only Redis failing.
    pub fn focus_pin_remove(&mut self, id: &FocusId, pin: &PinEntry) -> FocusResult<bool> {
        let args = [
            format!("pin:{}", pin.incarnation).into_bytes(),
            pin.raw.clone(),
        ];
        let reply = self.focus_script(
            &scripts::plain(scripts::PIN_REMOVE),
            &[self.keys.focus_key(id)],
            &args,
        )?;
        match status(&reply) {
            "removed" => Ok(true),
            "changed" => Ok(false),
            other => Err(unexpected("focus-pin-remove", other)),
        }
    }

    /// Reclaims the pins of one definition, archived or not: it examines at most its 32 pins,
    /// takes the session of each from the liveness rule and removes (compare-and-delete) those
    /// whose session is absent or replaced. A pin whose session is live or unknown, or that does
    /// not decode, is kept. The evidence was read by the caller, outside any atomic operation.
    /// Returns how many pins were removed; a definition that is not stored has none.
    ///
    /// # Errors
    ///
    /// Only Redis failing.
    pub fn focus_reclaim_pins(
        &mut self,
        id: &FocusId,
        evidence: &LivenessEvidence,
    ) -> FocusResult<usize> {
        let RawRead::Fields(fields) = self.focus_read_raw(id)? else {
            return Ok(0);
        };
        let mut removed = 0;
        for (field, raw) in fields.iter().filter(|(field, _)| field.starts_with("pin:")) {
            let Some(incarnation) = field.strip_prefix("pin:") else {
                continue;
            };
            let Ok(parsed) = incarnation.parse::<Incarnation>() else {
                continue;
            };
            if evidence
                .state_of(&parsed.machine, &parsed.session, None)
                .is_gone()
            {
                let entry = PinEntry {
                    incarnation: incarnation.to_owned(),
                    raw: raw.clone(),
                    value: None,
                };
                removed += usize::from(self.focus_pin_remove(id, &entry)?);
            }
        }
        Ok(removed)
    }
}
