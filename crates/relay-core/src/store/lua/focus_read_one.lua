-- KEYS: record. Every field of one definition, or `missing`, or `toolarge` when the record
-- holds too many fields or one is over the field bound: a record is never read unbounded.
local count = redis.call('HLEN', KEYS[1])
if count == 0 then return {'missing'} end
if count > MAX_RECORD_FIELDS then return {'toolarge'} end
for _, field in ipairs(redis.call('HKEYS', KEYS[1])) do
  if string.len(field) > MAX_FIELD_NAME_BYTES
    or redis.call('HSTRLEN', KEYS[1], field) > MAX_FIELD_BYTES then
    return {'toolarge'}
  end
end
local out = {'ok'}
local flat = redis.call('HGETALL', KEYS[1])
for i = 1, #flat do out[#out + 1] = flat[i] end
return out
