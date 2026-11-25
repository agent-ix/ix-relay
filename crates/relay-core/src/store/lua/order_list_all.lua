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
    members = redis.call('ZREVRANGEBYLEX', index, max, min, 'LIMIT', 0, 10001)
end
local orders = {}
local skipped = 0
for _, member in ipairs(members) do
    local split = string.find(member, '|', 1, true)
    if split then
        local enqueued = tonumber(string.sub(member, 1, split - 1))
        local order_id = string.sub(member, split + 1)
        local h = redis.call('HMGET', receipt_meta .. ':order:' .. order_id,
            'from', 'to', 'recipient', 'expect', 'ack_ms', 'enqueued_at',
            'delivered_at', 'acted_at', 'status')
        if h[1] and h[2] and h[3] and h[5] and h[6] and h[9]
            and tonumber(h[6]) == enqueued then
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
                expired = h[9] == 'expired',
            }
        else
            skipped = skipped + 1
        end
    else
        skipped = skipped + 1
    end
    if #orders > limit then break end
end

local complete, incomplete_reason = true, cjson.null
if not indexed then
    complete, incomplete_reason = false, 'index_not_started'
elseif since < oldest then
    complete = false
    incomplete_reason = indexed > now - tonumber(ARGV[6]) and 'before_index_start' or 'outside_retention'
end
if skipped > 0 then complete, incomplete_reason = false, 'unreadable_order_records' end
if complete and redis.call('HGET', receipt_meta, 'orders_all_senders_upgraded') ~= '1' then
    complete, incomplete_reason = false, 'sender_upgrade_required'
end

-- Orders still in the due set were written by some historical senders before they
-- learned to populate orders-all. Detect those gaps while they remain observable.
local due = receipt_meta .. ':orders-due'
if not readable(due, 'zset') then return redis.error_reply('order deadline index has wrong type') end
local missing = redis.call('ZRANGE', due, 0, 9999)
for _, order_id in ipairs(missing) do
    local enqueued = tonumber(redis.call('HGET', receipt_meta .. ':order:' .. order_id, 'enqueued_at') or '')
    if enqueued and enqueued >= since and enqueued <= until_at then
        local member = string.format('%013d|%s', enqueued, order_id)
        if not redis.call('ZSCORE', index, member) then
            complete, incomplete_reason = false, 'unindexed_open_orders'
            break
        end
    end
end

if #orders == 0 then orders = cjson.empty_array end
return cjson.encode({oldest_available_at = oldest or cjson.null, orders = orders,
    complete = complete, incomplete_reason = incomplete_reason, skipped = skipped})
