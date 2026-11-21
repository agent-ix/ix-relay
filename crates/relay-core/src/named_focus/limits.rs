// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The bounds of the named-focus stores (FR-061 "Bounds and retention"), one place.

/// Most selectors one revision holds: inclusions plus exclusions.
pub const MAX_SELECTORS: usize = 128;
/// Most bytes in one stable ID, project ID or reference.
pub const MAX_SELECTOR_BYTES: usize = 128;
/// Most bytes a member label keeps; longer labels are cut on a UTF-8 boundary.
pub const MAX_MEMBER_LABEL_BYTES: usize = 64;
/// Most bytes in a definition name.
pub const MAX_NAME_BYTES: usize = 128;
/// Most bytes in a definition description.
pub const MAX_DESCRIPTION_BYTES: usize = 512;
/// Most bytes in one encoded revision.
pub const MAX_REVISION_BYTES: usize = 56 * 1024;
/// Most bytes in any one stored field.
pub const MAX_FIELD_BYTES: usize = 64 * 1024;
/// Most non-archived definitions on one relay.
pub const MAX_ACTIVE_DEFINITIONS: usize = 128;
/// Most definitions on one relay, archived ones included.
pub const MAX_DEFINITIONS: usize = 256;
/// Revisions a definition keeps; an older one is deleted when a newer one is written.
pub const KEPT_REVISIONS: u64 = 8;
/// Most adopting incarnations a definition holds a pin for.
pub const MAX_PINS: usize = 32;
/// Most bytes in one encoded pin value.
pub const MAX_PIN_BYTES: usize = 512;
/// Most bytes in the reason a forced claim records.
pub const MAX_REASON_BYTES: usize = 256;
/// How long a definition stays archived before `create` may remove it: 90 days.
pub const ARCHIVE_RETENTION_MS: i64 = 90 * 24 * 60 * 60 * 1000;
/// Most archived definitions `create` examines when the definition limit would refuse it.
pub const REMOVAL_EXAMINED: usize = 8;
/// Most fields one definition record may hold: eight revisions, 32 pins, `meta`, `owner`,
/// and room for fields a newer writer adds.
pub const MAX_RECORD_FIELDS: usize = 128;
/// Most index entries a read of the indexes accepts before it refuses the store as damaged.
pub const MAX_INDEX_ENTRIES: usize = 1024;
/// Times a write re-reads and retries when a concurrent write changed an unrelated part of
/// the record between its read and its atomic operation.
pub const MAX_WRITE_ATTEMPTS: usize = 8;
