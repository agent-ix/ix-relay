-- Reads and validates the record under KEYS[1]; a record that does not decode is never
-- written. `refuse` answers with the status and the stored `meta` and `owner`; `refuse_moved`
-- adds the focus ID that holds the name now, or an empty string when no definition does.
local meta_raw = redis.call('HGET', KEYS[1], 'meta')
if not meta_raw then return {'not_found'} end
local owner_raw = redis.call('HGET', KEYS[1], 'owner')
local meta = decode(meta_raw)
local owner = decode(owner_raw)
if redis.call('HLEN', KEYS[1]) > MAX_RECORD_FIELDS or not meta or not owner
  or type(meta.focus_id) ~= 'string' or type(meta.state) ~= 'string'
  or type(meta.latest_revision) ~= 'number'
  or type(owner.owner_id) ~= 'string' or type(owner.epoch) ~= 'number' then
  return {'undecodable'}
end
local function refuse(status) return {status, meta_raw, owner_raw} end
local function name_moved(name_key)
  return name_key ~= '' and redis.call('HGET', KEYS[3], name_key) ~= meta.focus_id
end
local function refuse_moved(name_key)
  return {'name_moved', meta_raw, owner_raw, redis.call('HGET', KEYS[3], name_key) or ''}
end
