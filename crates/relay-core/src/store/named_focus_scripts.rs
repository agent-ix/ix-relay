// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Agent-IX
//! The server-side scripts of the named-focus store. Each restricted write re-reads the stored
//! record and compares before its first write, in one atomic operation (FR-061).
//!
//! The scripts only ever *read* JSON (to compare the latest revision, the owner and the
//! state); they never re-encode it. Every byte they store was encoded by the relay, so a
//! member this build does not define is neither dropped nor reordered. A write that was
//! planned from an older read of `meta` or `owner` answers `retry` instead of overwriting a
//! change it never saw.
//!
//! Replies are flat arrays of bulk strings: a status first, then the bytes the caller needs to
//! report the current state (`meta` and `owner` exactly as stored), so a refusal carries the
//! state the comparison saw and not a later read of it.

use crate::named_focus::limits::MAX_RECORD_FIELDS;

/// Helpers every script starts with.
const PRELUDE: &str = r"
local function decode(raw)
  if not raw then return nil end
  local ok, value = pcall(cjson.decode, raw)
  if ok and type(value) == 'table' then return value end
  return nil
end
local function has_prefix(text, prefix)
  return string.sub(text, 1, string.len(prefix)) == prefix
end
";

/// Reads and validates the record under `KEYS[1]`; a record that does not decode is never
/// written. `refuse` answers with the status and the stored `meta` and `owner`.
const GUARD: &str = r"
local meta_raw = redis.call('HGET', KEYS[1], 'meta')
if not meta_raw then return {'not_found'} end
local owner_raw = redis.call('HGET', KEYS[1], 'owner')
local meta = decode(meta_raw)
local owner = decode(owner_raw)
if redis.call('HLEN', KEYS[1]) > MAX_FIELDS or not meta or not owner
  or type(meta.focus_id) ~= 'string' or type(meta.state) ~= 'string'
  or type(meta.latest_revision) ~= 'number'
  or type(owner.owner_id) ~= 'string' or type(owner.epoch) ~= 'number' then
  return {'undecodable'}
end
local function refuse(status) return {status, meta_raw, owner_raw} end
local function name_moved(name_key)
  return name_key ~= '' and redis.call('HGET', KEYS[3], name_key) ~= meta.focus_id
end
";

/// `KEYS`: record, definitions index, names index.
/// `ARGV`: expected revision, caller owner ID, expected epoch, `meta` as read, `owner` as read,
/// new `meta`, new `owner`, new revision, its number, new name key, new index value, the name
/// key the target was resolved by (or empty), revisions kept, old name key.
pub(super) const UPDATE: &str = r"
if name_moved(ARGV[12]) then return refuse('name_moved') end
if meta.state == 'archived' then return refuse('archived') end
if owner.owner_id ~= ARGV[2] then return refuse('not_owner') end
if owner.epoch ~= tonumber(ARGV[3]) then return refuse('owner_epoch_conflict') end
if meta.latest_revision ~= tonumber(ARGV[1]) then return refuse('revision_conflict') end
if ARGV[10] ~= ARGV[14] then
  local holder = redis.call('HGET', KEYS[3], ARGV[10])
  if holder and holder ~= meta.focus_id then return {'name_taken', holder} end
end
if meta_raw ~= ARGV[4] or owner_raw ~= ARGV[5] then return {'retry'} end
redis.call('HSET', KEYS[1], 'rev:' .. ARGV[9], ARGV[8], 'meta', ARGV[6], 'owner', ARGV[7])
if ARGV[10] ~= ARGV[14] and redis.call('HGET', KEYS[3], ARGV[14]) == meta.focus_id then
  redis.call('HDEL', KEYS[3], ARGV[14])
end
redis.call('HSET', KEYS[3], ARGV[10], meta.focus_id)
redis.call('HSET', KEYS[2], meta.focus_id, ARGV[11])
local newest = tonumber(ARGV[9])
local kept = tonumber(ARGV[13])
for _, field in ipairs(redis.call('HKEYS', KEYS[1])) do
  if has_prefix(field, 'rev:') then
    local number = tonumber(string.sub(field, 5))
    if number and number <= newest - kept then redis.call('HDEL', KEYS[1], field) end
  end
end
return {'ok'}
";

/// `ARGV`: expected revision, caller owner ID, expected epoch, `meta` as read, `owner` as read,
/// new `meta`, new `owner`, new index value, the name key the target was resolved by (or
/// empty), the definition's name key.
pub(super) const ARCHIVE: &str = r"
if name_moved(ARGV[9]) then return refuse('name_moved') end
if meta.state == 'archived' then return refuse('archived') end
if owner.owner_id ~= ARGV[2] then return refuse('not_owner') end
if owner.epoch ~= tonumber(ARGV[3]) then return refuse('owner_epoch_conflict') end
if meta.latest_revision ~= tonumber(ARGV[1]) then return refuse('revision_conflict') end
if meta_raw ~= ARGV[4] or owner_raw ~= ARGV[5] then return {'retry'} end
redis.call('HSET', KEYS[1], 'meta', ARGV[6], 'owner', ARGV[7])
redis.call('HSET', KEYS[2], meta.focus_id, ARGV[8])
if redis.call('HGET', KEYS[3], ARGV[10]) == meta.focus_id then
  redis.call('HDEL', KEYS[3], ARGV[10])
end
return {'ok'}
";

/// `ARGV`: observed owner ID, expected epoch, `meta` as read, `owner` as read, new `owner`, the
/// name key the target was resolved by (or empty). The caller's own ID is never compared.
pub(super) const CLAIM: &str = r"
if name_moved(ARGV[6]) then return refuse('name_moved') end
if owner.owner_id ~= ARGV[1] or owner.epoch ~= tonumber(ARGV[2]) then
  return refuse('owner_epoch_conflict')
end
if meta_raw ~= ARGV[3] or owner_raw ~= ARGV[4] then return {'retry'} end
redis.call('HSET', KEYS[1], 'owner', ARGV[5])
return {'ok'}
";

/// `ARGV`: caller owner ID, expected epoch, `meta` as read, `owner` as read, new `owner`, the
/// name key the target was resolved by (or empty).
pub(super) const TRANSFER: &str = r"
if name_moved(ARGV[6]) then return refuse('name_moved') end
if meta.state == 'archived' then return refuse('archived') end
if owner.owner_id ~= ARGV[1] then return refuse('not_owner') end
if owner.epoch ~= tonumber(ARGV[2]) then return refuse('owner_epoch_conflict') end
if meta_raw ~= ARGV[3] or owner_raw ~= ARGV[4] then return {'retry'} end
redis.call('HSET', KEYS[1], 'owner', ARGV[5])
return {'ok'}
";

/// `KEYS`: record, definitions index, names index. `ARGV`: name key, `meta`, `owner`, revision 1,
/// index value, focus ID, non-archived limit, total limit, now (ms), retention (ms), record key
/// stem, then the removal candidates, oldest archived first.
pub(super) const CREATE: &str = r"
if redis.call('EXISTS', KEYS[1]) == 1 then return {'exists'} end
local holder = redis.call('HGET', KEYS[3], ARGV[1])
if holder then return {'name_taken', holder} end
if redis.call('HLEN', KEYS[2]) > 4096 then return {'undecodable'} end
local entries = redis.call('HGETALL', KEYS[2])
local active, total = 0, 0
for i = 1, #entries, 2 do
  total = total + 1
  local entry = decode(entries[i + 1])
  if not (entry and type(entry.archived_at_ms) == 'number') then active = active + 1 end
end
if active >= tonumber(ARGV[7]) then return {'limit_active'} end
local removed = ''
if total >= tonumber(ARGV[8]) then
  local cutoff = tonumber(ARGV[9]) - tonumber(ARGV[10])
  for i = 12, #ARGV do
    local candidate = ARGV[i]
    local entry = decode(redis.call('HGET', KEYS[2], candidate))
    if entry and type(entry.archived_at_ms) == 'number' and entry.archived_at_ms <= cutoff then
      local key = ARGV[11] .. candidate
      local pinned = redis.call('HLEN', key) > 4096
      if not pinned then
        for _, field in ipairs(redis.call('HKEYS', key)) do
          if has_prefix(field, 'pin:') then pinned = true break end
        end
      end
      if not pinned then
        redis.call('DEL', key)
        redis.call('HDEL', KEYS[2], candidate)
        removed = candidate
        total = total - 1
        break
      end
    end
  end
  if total >= tonumber(ARGV[8]) then return {'limit_total'} end
end
redis.call('HSET', KEYS[1], 'meta', ARGV[2], 'owner', ARGV[3], 'rev:1', ARGV[4])
redis.call('HSET', KEYS[2], ARGV[6], ARGV[5])
redis.call('HSET', KEYS[3], ARGV[1], ARGV[6])
return {'ok', removed}
";

/// `KEYS`: record. `ARGV`: pin field, pin value, most pins.
pub(super) const PIN_PUT: &str = r"
if meta.state == 'archived' then return refuse('archived') end
local count, exists = 0, false
for _, field in ipairs(redis.call('HKEYS', KEYS[1])) do
  if has_prefix(field, 'pin:') then
    count = count + 1
    if field == ARGV[1] then exists = true end
  end
end
if not exists and count >= tonumber(ARGV[3]) then return {'limit'} end
redis.call('HSET', KEYS[1], ARGV[1], ARGV[2])
return {'ok'}
";

/// `KEYS`: record. `ARGV`: pin field, the value the caller read. Removes only that value.
pub(super) const PIN_REMOVE: &str = r"
if redis.call('HGET', KEYS[1], ARGV[1]) == ARGV[2] then
  redis.call('HDEL', KEYS[1], ARGV[1])
  return {'removed'}
end
return {'changed'}
";

/// `KEYS`: record. Every field of one definition, or `missing`, or `toolarge` when the record
/// holds too many fields or one is over 64 KiB: a record is never read unbounded.
pub(super) const READ_ONE: &str = r"
local count = redis.call('HLEN', KEYS[1])
if count == 0 then return {'missing'} end
if count > MAX_FIELDS then return {'toolarge'} end
for _, field in ipairs(redis.call('HKEYS', KEYS[1])) do
  if string.len(field) > 1024 or redis.call('HSTRLEN', KEYS[1], field) > 65536 then
    return {'toolarge'}
  end
end
local out = {'ok'}
local flat = redis.call('HGETALL', KEYS[1])
for i = 1, #flat do out[#out + 1] = flat[i] end
return out
";

/// `KEYS`: definitions index. `ARGV`: record key stem. For each definition: its ID, `meta`,
/// `owner` and latest revision; a field that is missing or over 64 KiB is an empty string.
pub(super) const LIST: &str = r"
if redis.call('HLEN', KEYS[1]) > 1024 then return {'toolarge'} end
local out = {'ok'}
for _, id in ipairs(redis.call('HKEYS', KEYS[1])) do
  local key = ARGV[1] .. id
  local meta, owner, revision = '', '', ''
  if redis.call('HSTRLEN', key, 'meta') <= 65536 then
    meta = redis.call('HGET', key, 'meta') or ''
  end
  if redis.call('HSTRLEN', key, 'owner') <= 65536 then
    owner = redis.call('HGET', key, 'owner') or ''
  end
  local decoded = decode(meta)
  if decoded and type(decoded.latest_revision) == 'number' then
    local field = string.format('rev:%.0f', decoded.latest_revision)
    if redis.call('HSTRLEN', key, field) <= 65536 then
      revision = redis.call('HGET', key, field) or ''
    end
  end
  out[#out + 1] = id
  out[#out + 1] = meta
  out[#out + 1] = owner
  out[#out + 1] = revision
end
return out
";

/// The scripts that operate on an existing record, each behind the helpers and the guard.
pub(super) fn guarded(body: &str) -> String {
    compose(&[PRELUDE, GUARD, body])
}

/// A script with only the helpers.
pub(super) fn plain(body: &str) -> String {
    compose(&[PRELUDE, body])
}

fn compose(parts: &[&str]) -> String {
    parts
        .concat()
        .replace("MAX_FIELDS", &MAX_RECORD_FIELDS.to_string())
}
