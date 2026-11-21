-- KEYS: record. ARGV: pin field, pin value, most pins.
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
