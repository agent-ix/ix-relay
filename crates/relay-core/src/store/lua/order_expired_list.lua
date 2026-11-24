-- Reads the orders whose AckExpired is recorded and that were not Acted, newest
-- expiry first, without writing anything.
-- ARGV: cap, scan cap, fleet lane lists hash, receipt stream, receipt metadata.
-- A recent rank trim of the index also sets truncated.
-- Returns {truncated, indexed_since, trimmed_at, lane list count, the lane lists as stored,
-- then per order: id, expired_at, to,
-- expect, enqueued_at, delivered_at, ack_ms, last_state, escalation}.
if not receipt_ready() then return redis.error_reply('receipt history has wrong type') end
local cap, scan_cap = tonumber(ARGV[1]), tonumber(ARGV[2])
local index = receipt_meta .. ':orders-expired'
local lists = readable(ARGV[3], 'hash') and redis.call('HVALS', ARGV[3]) or {}
local out = {0, redis.call('HGET', receipt_meta, 'orders_expired_since_ms') or '',
    redis.call('HGET', receipt_meta, 'orders_expired_trim_ms') or '', #lists}
for _, list in ipairs(lists) do out[#out + 1] = list end
if out[3] ~= '' and tonumber(out[3]) > now_ms('') - 691200000 then out[1] = 1 end
local kept, from, chunk = 0, 0, 100
while from < scan_cap do
    local rows = redis.call('ZREVRANGE', index, from, from + chunk - 1, 'WITHSCORES')
    if #rows == 0 then return out end
    for i = 1, #rows, 2 do
        local h = redis.call('HMGET', order_key(rows[i]), 'to', 'expect', 'enqueued_at',
            'delivered_at', 'ack_ms', 'last_state', 'acted_at', 'escalation', 'status')
        if h[1] and h[9] == 'expired' and not h[7] then
            if kept == cap then
                out[1] = 1
                return out
            end
            kept = kept + 1
            out[#out + 1] = rows[i]
            out[#out + 1] = string.format('%.0f', tonumber(rows[i + 1]))
            for _, f in ipairs({h[1], h[2], h[3], h[4], h[5], h[6], h[8]}) do
                out[#out + 1] = f or ''
            end
        end
    end
    from = from + chunk
end
if redis.call('ZCARD', index) > scan_cap then out[1] = 1 end
return out
