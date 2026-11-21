// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The server-side scripts of the named-focus store. Each restricted write re-reads the stored
//! record and compares before its first write, in one atomic operation (FR-061).
//!
//! The scripts live in `lua/focus_*.lua`, one `LazyLock<Script>` each, like the other stores'.
//! They only ever *read* JSON (to compare the latest revision, the owner and the state); they
//! never re-encode it. Every byte they store was encoded by the relay, so a member this build
//! does not define is neither dropped nor reordered. A write that was planned from an older
//! read of `meta` or `owner` answers `retry` instead of overwriting a change it never saw.
//!
//! The bounds the scripts check are the Rust limits in [`crate::named_focus::limits`], declared
//! as Lua locals ahead of the script text; none is spelled twice.
//!
//! Replies are flat arrays of bulk strings: a status first, then the bytes the caller needs to
//! report the current state (`meta` and `owner` exactly as stored), so a refusal carries the
//! state the comparison saw and not a later read of it.

use std::sync::LazyLock;

use redis::Script;

use crate::named_focus::limits::{
    MAX_FIELD_BYTES, MAX_FIELD_NAME_BYTES, MAX_INDEX_ENTRIES, MAX_RECORD_FIELDS,
};

/// The bounds, declared as Lua locals.
fn bounds() -> String {
    format!(
        "local MAX_RECORD_FIELDS, MAX_FIELD_BYTES, MAX_FIELD_NAME_BYTES, MAX_INDEX_ENTRIES \
         = {MAX_RECORD_FIELDS}, {MAX_FIELD_BYTES}, {MAX_FIELD_NAME_BYTES}, {MAX_INDEX_ENTRIES}\n"
    )
}

/// A script with the bounds and the helpers.
fn plain(body: &str) -> Script {
    Script::new(&[&bounds(), include_str!("lua/focus_prelude.lua"), body].concat())
}

/// A script that operates on an existing record: behind the helpers and the guard.
fn guarded(body: &str) -> Script {
    Script::new(
        &[
            &bounds(),
            include_str!("lua/focus_prelude.lua"),
            include_str!("lua/focus_guard.lua"),
            body,
        ]
        .concat(),
    )
}

pub(super) static UPDATE: LazyLock<Script> =
    LazyLock::new(|| guarded(include_str!("lua/focus_update.lua")));
pub(super) static ARCHIVE: LazyLock<Script> =
    LazyLock::new(|| guarded(include_str!("lua/focus_archive.lua")));
pub(super) static CLAIM: LazyLock<Script> =
    LazyLock::new(|| guarded(include_str!("lua/focus_claim.lua")));
pub(super) static TRANSFER: LazyLock<Script> =
    LazyLock::new(|| guarded(include_str!("lua/focus_transfer.lua")));
pub(super) static PIN_PUT: LazyLock<Script> =
    LazyLock::new(|| guarded(include_str!("lua/focus_pin_put.lua")));
pub(super) static CREATE: LazyLock<Script> =
    LazyLock::new(|| plain(include_str!("lua/focus_create.lua")));
pub(super) static PIN_REMOVE: LazyLock<Script> =
    LazyLock::new(|| plain(include_str!("lua/focus_pin_remove.lua")));
pub(super) static READ_ONE: LazyLock<Script> =
    LazyLock::new(|| plain(include_str!("lua/focus_read_one.lua")));
pub(super) static LIST: LazyLock<Script> =
    LazyLock::new(|| plain(include_str!("lua/focus_list.lua")));
