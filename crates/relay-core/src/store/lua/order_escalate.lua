-- Files the escalation of an expired order once.
-- KEYS: esc seq, esc index. ARGV: original, escalation key prefix, meta JSON
-- (an object, its `id` added here), status, created_at_ms, receipt stream, receipt metadata.
local okey = order_key(ARGV[1])
local pending = receipt_meta .. ':orders-escalate'
if redis.call('HEXISTS', okey, 'escalation') == 1 or redis.call('EXISTS', okey) == 0 then
    redis.call('ZREM', pending, ARGV[1])
    return 0
end
local n = redis.call('INCR', KEYS[1])
local id = 'e' .. string.format('%d', n)
local meta = '{"id":' .. cjson.encode(id) .. ',' .. string.sub(ARGV[3], 2)
redis.call('HSET', ARGV[2] .. id, 'meta', meta, 'status', ARGV[4], 'decision', '')
redis.call('ZADD', KEYS[2], tonumber(ARGV[5]), id)
redis.call('HSET', okey, 'escalation', id)
redis.call('ZREM', pending, ARGV[1])
return 1
