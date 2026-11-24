-- Records AckExpired for orders past their deadline and returns the orders whose
-- escalation is still to be filed.
-- ARGV: limit, clock override, receipt stream, receipt metadata.
if not receipt_ready() then return redis.error_reply('receipt history has wrong type') end
local now = now_ms(ARGV[2])
local due = receipt_meta .. ':orders-due'
local pending = receipt_meta .. ':orders-escalate'
for _, original in ipairs(redis.call('ZRANGEBYSCORE', due, '-inf', now, 'LIMIT', 0, tonumber(ARGV[1]))) do
    local okey = order_key(original)
    local h = redis.call('HMGET', okey, 'from', 'recipient', 'state', 'expect', 'status')
    if h[5] ~= 'open' then
        redis.call('ZREM', due, original)
    else
        receipt_emit({
            original = original, idpart = 'order', outcome = 'ack_expired',
            sender = h[1], target = h[2], now = now, reason = 'ack_deadline',
            expect = h[4] ~= '' and h[4] or nil,
        })
        redis.call('HSET', okey, 'last_state', h[3])
        order_close(original, 'expired')
        redis.call('ZADD', pending, now, original)
    end
end
local out = {}
for _, original in ipairs(redis.call('ZRANGE', pending, 0, 19)) do
    local h = redis.call('HMGET', order_key(original), 'from', 'to', 'recipient', 'last_state', 'expect')
    out[#out + 1] = {original, h[1] or '', h[2] or '', h[3] or '', h[4] or '', h[5] or ''}
end
return out
