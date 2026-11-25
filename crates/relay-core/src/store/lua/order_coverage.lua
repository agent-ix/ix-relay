-- Confirms, clears, or reads the all-order coverage boundary.
-- ARGV: action, receipt stream, receipt metadata.
if not receipt_ready() then return redis.error_reply('receipt history has wrong type') end
local action = ARGV[1]
if action == 'confirm' then
    local now = now_ms('')
    redis.call('HSET', receipt_meta, 'orders_all_complete_since_ms', now)
    return tostring(now)
elseif action == 'clear' then
    return tostring(redis.call('HDEL', receipt_meta, 'orders_all_complete_since_ms'))
elseif action == 'show' then
    return redis.call('HGET', receipt_meta, 'orders_all_complete_since_ms') or ''
end
return redis.error_reply('invalid order coverage action')
