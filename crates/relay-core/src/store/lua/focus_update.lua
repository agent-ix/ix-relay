-- KEYS: record, definitions index, names index.
-- ARGV: expected revision, caller owner ID, expected epoch, `meta` as read, `owner` as read,
-- new `meta`, new `owner`, new revision, its number, new name key, new index value, the name
-- key the target was resolved by (or empty), revisions kept, old name key.
if name_moved(ARGV[12]) then return refuse_moved(ARGV[12]) end
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
