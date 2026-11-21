-- KEYS: record, definitions index, names index.
-- ARGV: expected revision, caller owner ID, expected epoch, `meta` as read, `owner` as read,
-- new `meta`, new `owner`, new index value, the name key the target was resolved by (or
-- empty), the definition's name key.
if name_moved(ARGV[9]) then return refuse_moved(ARGV[9]) end
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
