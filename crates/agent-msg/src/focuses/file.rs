// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The definition file of `focuses create` and `focuses update` (FR-061).
//!
//! The file is the caller's input and is read strictly: a member the relay does not define (a
//! focus reference, a second team, a misspelled key) is refused as an invalid argument, naming
//! it. Stored records are read tolerantly; this strictness belongs to the file only.
//!
//! ```json
//! {
//!   "team": "SWM",
//!   "name": "Completion",
//!   "description": "optional",
//!   "include": {
//!     "initiatives": ["Completion"],
//!     "projects": [],
//!     "milestones": [{"ref": "Beta", "project": "Alpha"}],
//!     "tickets": ["SWM-12"]
//!   },
//!   "exclude": {}
//! }
//! ```
//!
//! A member is a reference string (stable ID, identifier or exact name) or an object
//! `{"ref": "...", "label": "..."}`, a milestone also `"project"`. `focuses show --json` prints
//! exactly this shape as its `definition`, with canonical IDs as references, so it can be saved
//! and used as an update file. The label only displays; it is never read back as identity.

use std::{fs::File, io::Read, path::Path};

use relay_core::named_focus::{FocusError, FocusResult, limits::MAX_SELECTOR_BYTES};
use serde_json::{Map, Value};

/// Most bytes a definition file may have: far above any file within the relay's bounds.
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// One member of a file: a reference to resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileMember {
    /// The stable ID, identifier or exact name.
    pub(crate) reference: String,
    /// For a milestone, the project that must own it.
    pub(crate) project: Option<String>,
}

/// The four kinds of members of one list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileMembers {
    pub(crate) initiatives: Vec<FileMember>,
    pub(crate) projects: Vec<FileMember>,
    pub(crate) milestones: Vec<FileMember>,
    pub(crate) tickets: Vec<FileMember>,
}

impl FileMembers {
    fn len(&self) -> usize {
        self.initiatives.len() + self.projects.len() + self.milestones.len() + self.tickets.len()
    }
}

/// A parsed definition file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DefinitionFile {
    pub(crate) team: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) include: FileMembers,
    pub(crate) exclude: FileMembers,
}

/// Reads and parses the file at `path`, refusing one over the byte bound.
pub(crate) fn read_definition(path: &Path) -> FocusResult<DefinitionFile> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| FocusError::invalid("file", format!("{}: {error}", path.display())))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(FocusError::Limit {
            bound: "definition file bytes",
            max: MAX_FILE_BYTES,
        });
    }
    parse_definition(&bytes)
}

fn refuse<T>(path: &str, message: impl Into<String>) -> FocusResult<T> {
    Err(FocusError::invalid(format!("file {path}"), message))
}

fn text(path: &str, value: &Value) -> FocusResult<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        _ => refuse(path, "must be a string"),
    }
}

fn object<'a>(path: &str, value: &'a Value) -> FocusResult<&'a Map<String, Value>> {
    match value {
        Value::Object(map) => Ok(map),
        _ => refuse(path, "must be a JSON object"),
    }
}

fn only_keys(path: &str, map: &Map<String, Value>, allowed: &[&str]) -> FocusResult<()> {
    match map.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => refuse(
            format!("{path}.{key}").trim_start_matches('.'),
            format!(
                "is not a member the relay defines (allowed here: {}); a focus reference, a \
                 second team or a misspelled key is refused",
                allowed.join(", ")
            ),
        ),
        None => Ok(()),
    }
}

fn member(path: &str, value: &Value, milestone: bool) -> FocusResult<FileMember> {
    match value {
        Value::String(reference) => Ok(FileMember {
            reference: reference.clone(),
            project: None,
        }),
        Value::Object(map) => {
            let allowed: &[&str] = if milestone {
                &["ref", "project", "label"]
            } else {
                &["ref", "label"]
            };
            only_keys(path, map, allowed)?;
            let reference = text(
                &format!("{path}.ref"),
                map.get("ref")
                    .ok_or_else(|| FocusError::invalid(format!("file {path}"), "needs a ref"))?,
            )?;
            let project = map
                .get("project")
                .map(|value| text(&format!("{path}.project"), value))
                .transpose()?;
            if let Some(label) = map.get("label") {
                text(&format!("{path}.label"), label)?;
            }
            Ok(FileMember { reference, project })
        }
        _ => refuse(path, "must be a reference string or an object with a ref"),
    }
}

fn members(path: &str, value: &Value) -> FocusResult<FileMembers> {
    let map = object(path, value)?;
    only_keys(
        path,
        map,
        &["initiatives", "projects", "milestones", "tickets"],
    )?;
    let list = |key: &str, milestone: bool| -> FocusResult<Vec<FileMember>> {
        let Some(value) = map.get(key) else {
            return Ok(Vec::new());
        };
        let Value::Array(items) = value else {
            return refuse(&format!("{path}.{key}"), "must be an array");
        };
        items
            .iter()
            .enumerate()
            .map(|(index, item)| member(&format!("{path}.{key}[{index}]"), item, milestone))
            .collect()
    };
    Ok(FileMembers {
        initiatives: list("initiatives", false)?,
        projects: list("projects", false)?,
        milestones: list("milestones", true)?,
        tickets: list("tickets", false)?,
    })
}

/// Parses a definition file.
pub(crate) fn parse_definition(bytes: &[u8]) -> FocusResult<DefinitionFile> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| FocusError::invalid("file", format!("not valid JSON: {error}")))?;
    let root = object("", &value)?;
    only_keys(
        "",
        root,
        &["team", "name", "description", "include", "exclude"],
    )?;
    let required = |key: &str| {
        root.get(key)
            .ok_or_else(|| FocusError::invalid(format!("file {key}"), "is required"))
    };
    let include = members("include", required("include")?)?;
    let exclude = root
        .get("exclude")
        .map(|value| members("exclude", value))
        .transpose()?
        .unwrap_or_default();
    let definition = DefinitionFile {
        team: text("team", required("team")?)?,
        name: text("name", required("name")?)?,
        description: root
            .get("description")
            .map(|value| text("description", value))
            .transpose()?
            .unwrap_or_default(),
        include,
        exclude,
    };
    definition.check_bounds()?;
    Ok(definition)
}

impl DefinitionFile {
    /// The bounds that can be checked before anything is resolved: too many selectors, or a
    /// reference longer than a stable ID may be.
    pub(crate) fn check_bounds(&self) -> FocusResult<()> {
        let total = self.include.len() + self.exclude.len();
        if total > relay_core::named_focus::limits::MAX_SELECTORS {
            return Err(FocusError::limit(
                "selectors per revision",
                relay_core::named_focus::limits::MAX_SELECTORS,
            ));
        }
        for list in [&self.include, &self.exclude] {
            for member in list
                .initiatives
                .iter()
                .chain(&list.projects)
                .chain(&list.milestones)
                .chain(&list.tickets)
            {
                for value in std::iter::once(&member.reference).chain(&member.project) {
                    if value.len() > MAX_SELECTOR_BYTES {
                        return Err(FocusError::limit(
                            "bytes per stable ID or selector",
                            MAX_SELECTOR_BYTES,
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
