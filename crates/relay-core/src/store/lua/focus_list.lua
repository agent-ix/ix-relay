-- KEYS: definitions index. ARGV: record key stem. For each definition: its ID, `meta`, `owner`
-- and latest revision; a field that is missing or over the field bound is an empty string.
if redis.call('HLEN', KEYS[1]) > MAX_INDEX_ENTRIES then return {'toolarge'} end
local out = {'ok'}
for _, id in ipairs(redis.call('HKEYS', KEYS[1])) do
  local key = ARGV[1] .. id
  local meta, owner, revision = '', '', ''
  if redis.call('HSTRLEN', key, 'meta') <= MAX_FIELD_BYTES then
    meta = redis.call('HGET', key, 'meta') or ''
  end
  if redis.call('HSTRLEN', key, 'owner') <= MAX_FIELD_BYTES then
    owner = redis.call('HGET', key, 'owner') or ''
  end
  local decoded = decode(meta)
  if decoded and type(decoded.latest_revision) == 'number' then
    local field = string.format('rev:%.0f', decoded.latest_revision)
    if redis.call('HSTRLEN', key, field) <= MAX_FIELD_BYTES then
      revision = redis.call('HGET', key, field) or ''
    end
  end
  out[#out + 1] = id
  out[#out + 1] = meta
  out[#out + 1] = owner
  out[#out + 1] = revision
end
return out
