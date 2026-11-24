-- Counts a failed attempt to file an order's escalation; at the limit the order
-- leaves the pending set for the failed set, so it stops blocking the rest.
-- ARGV: original, attempts allowed, receipt stream, receipt metadata.
local okey = order_key(ARGV[1])
if redis.call('EXISTS', okey) == 0 or redis.call('HEXISTS', okey, 'escalation') == 1 then
    redis.call('ZREM', receipt_meta .. ':orders-escalate', ARGV[1])
    return 0
end
local failures = redis.call('HINCRBY', okey, 'esc_failures', 1)
if failures >= tonumber(ARGV[2]) then
    redis.call('HSET', okey, 'escalation', 'failed')
    redis.call('ZREM', receipt_meta .. ':orders-escalate', ARGV[1])
    -- Counted per host: the server part of the recipient lane address.
    local host = string.match(redis.call('HGET', okey, 'to') or '', '^([^:]*):') or ''
    local failed = receipt_meta .. ':orders-escalate-failed:' .. host
    local at = now_ms('')
    redis.call('ZADD', failed, at, ARGV[1])
    redis.call('ZREMRANGEBYSCORE', failed, '-inf', at - 604800000)
    redis.call('ZREMRANGEBYRANK', failed, 0, -1001)
end
return failures
