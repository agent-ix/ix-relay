-- KEYS: record, definitions index, names index.
-- ARGV: observed owner ID, expected epoch, `meta` as read, `owner` as read, new `owner`, the
-- name key the target was resolved by (or empty). The caller's own ID is never compared.
if name_moved(ARGV[6]) then return refuse_moved(ARGV[6]) end
if owner.owner_id ~= ARGV[1] or owner.epoch ~= tonumber(ARGV[2]) then
  return refuse('owner_epoch_conflict')
end
if meta_raw ~= ARGV[3] or owner_raw ~= ARGV[4] then return {'retry'} end
redis.call('HSET', KEYS[1], 'owner', ARGV[5])
return {'ok'}
