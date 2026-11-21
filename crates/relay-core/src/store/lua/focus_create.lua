-- KEYS: record, definitions index, names index.
-- ARGV: name key, `meta`, `owner`, revision 1, index value, focus ID, non-archived limit,
-- total limit, now (ms), retention (ms), record key stem, then the removal candidates, oldest
-- archived first.
if redis.call('EXISTS', KEYS[1]) == 1 then return {'exists'} end
local holder = redis.call('HGET', KEYS[3], ARGV[1])
if holder then return {'name_taken', holder} end
if redis.call('HLEN', KEYS[2]) > MAX_INDEX_ENTRIES then return {'undecodable'} end
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
      local pinned = redis.call('HLEN', key) > MAX_RECORD_FIELDS
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
