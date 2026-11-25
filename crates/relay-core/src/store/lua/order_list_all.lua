-- Reads every retained order outcome within an inclusive enqueue-time window.
-- ARGV: since ms, until ms, page size, cursor time, cursor order id, retention ms,
-- receipt stream, receipt metadata.
-- All index members share score 0 and sort lexically by padded timestamp and stable id.
if not receipt_ready() then return redis.error_reply('receipt history has wrong type') end
local index = receipt_meta .. ':orders-all'
if not readable(index, 'zset') then return redis.error_reply('order history has wrong type') end
local since, until_at, limit = tonumber(ARGV[1]), tonumber(ARGV[2]), tonumber(ARGV[3])
local now = now_ms('')
local indexed = tonumber(redis.call('HGET', receipt_meta, 'orders_all_since_ms') or '')
local oldest = indexed and math.max(indexed, now - tonumber(ARGV[6])) or nil
local lower = math.max(since, oldest or since)
local members = {}
if lower <= until_at then
    local min = '[' .. string.format('%013d|', lower)
    local max = '[' .. string.format('%013d|', until_at + 1)
    if ARGV[4] ~= '' then
        max = '(' .. string.format('%013d|%s', tonumber(ARGV[4]), ARGV[5])
    end
    members = redis.call('ZREVRANGEBYLEX', index, max, min, 'LIMIT', 0, limit + 1)
end
local orders = {}
for _, member in ipairs(members) do
    local split = string.find(member, '|', 1, true)
    if not split then return redis.error_reply('order history index is malformed') end
    local enqueued = tonumber(string.sub(member, 1, split - 1))
    local order_id = string.sub(member, split + 1)
    local h = redis.call('HMGET', receipt_meta .. ':order:' .. order_id,
        'from', 'to', 'recipient', 'expect', 'ack_ms', 'enqueued_at',
        'delivered_at', 'acted_at', 'status')
    if not h[1] or not h[2] or not h[3] or not h[5] or not h[6] or not h[9] then
        return redis.error_reply('order history record is unavailable')
    end
    if tonumber(h[6]) ~= enqueued then
        return redis.error_reply('order history record does not match its index')
    end
    orders[#orders + 1] = {
        order_id = order_id,
        sender = h[1],
        recipient = h[2],
        receipt_target = h[3],
        expect_ticket = h[4] ~= '' and h[4] or cjson.null,
        ack_ms = tonumber(h[5]),
        enqueued_at = enqueued,
        delivered_at = h[7] and tonumber(h[7]) or cjson.null,
        acted_at = h[8] and tonumber(h[8]) or cjson.null,
        state = h[9],
    }
end
return cjson.encode({oldest_available_at = oldest or cjson.null, orders = orders})
