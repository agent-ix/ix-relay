-- Receipt keys are the final two ARGV values of an instrumented script.
local receipt_stream, receipt_meta = ARGV[#ARGV - 1], ARGV[#ARGV]

receipt_ready = function()
    return readable(receipt_stream, 'stream') and readable(receipt_meta, 'hash')
end
receipt_gap = function(now)
    local first = tonumber(redis.call('HGET', receipt_meta, 'gap_start'))
    local last = tonumber(redis.call('HGET', receipt_meta, 'gap_end'))
    redis.call('HSET', receipt_meta, 'gap_start', math.min(first or now, now),
        'gap_end', math.max(last or now, now))
end

local function order_key(original) return receipt_meta .. ':order:' .. original end

-- Appends one receipt event from explicit fields (`f`): used for transitions of
-- an entry and for the order and activity receipts, which have no entry at hand.
receipt_emit = function(f)
    local now, outcome = f.now, f.outcome
    redis.call('HSETNX', receipt_meta, 'instrumentation_start_ms', now)
    local attempt = f.attempt or cjson.null
    local event_id = f.original .. '/' .. f.idpart .. '/' .. outcome
    if redis.call('HEXISTS', receipt_meta, 'event:' .. event_id) == 1 then return false end
    local event = {
        event_id = event_id,
        original_id = f.original, attempt_id = attempt, outcome = outcome,
        sender = f.sender, target = f.target,
        event_time_ms = outcome == 'delivered' and cjson.null or now,
        observed_at_ms = now, evidence = f.evidence or cjson.null,
        reason = f.reason or cjson.null,
        session = f.session, expect_ticket = f.expect, delivered_at_ms = f.delivered_at,
    }
    local okey = order_key(f.original)
    if redis.call('EXISTS', okey) == 1 then redis.call('HSET', okey, 'state', outcome) end
    local cursor = redis.call('XADD', receipt_stream, '*', 'event', cjson.encode(event))
    redis.call('HSET', receipt_meta, 'event:' .. event_id, cursor, 'last_cursor', cursor)
    local count = redis.call('XLEN', receipt_stream)
    if count > 10000 then
        local evicted = redis.call('XRANGE', receipt_stream, '-', '+', 'COUNT', count - 10000)
        for _, row in ipairs(evicted) do
            local valid, old = pcall(cjson.decode, row[2][2])
            if not valid or type(old) ~= 'table' or type(old.observed_at_ms) ~= 'number'
                or type(old.event_id) ~= 'string' then
                receipt_gap(now)
                old = {observed_at_ms = now}
            else
                redis.call('HDEL', receipt_meta, 'event:' .. old.event_id)
            end
            local boundary = tonumber(redis.call('HGET', receipt_meta, 'evicted_through_ms'))
            redis.call('HSET', receipt_meta, 'evicted_through_ms',
                math.max(boundary or old.observed_at_ms, old.observed_at_ms),
                'evicted_through_cursor', row[1])
        end
        redis.call('XTRIM', receipt_stream, 'MAXLEN', 10000)
    end
    return true
end

-- Closes an open order (`status` becomes the reason), dropping it from the
-- deadline index and the per-recipient unacted count. False when it was not open.
order_close = function(original, status)
    local okey = order_key(original)
    if not readable(okey, 'hash') then
        redis.call('HDEL', receipt_meta, 'orders_all_complete_since_ms')
        return false
    end
    local stored_status = redis.call('HGET', okey, 'status')
    if not stored_status then
        redis.call('HDEL', receipt_meta, 'orders_all_complete_since_ms')
        return false
    end
    if stored_status ~= 'open' then return false end
    local enqueued = tonumber(redis.call('HGET', okey, 'enqueued_at') or '')
    local all = receipt_meta .. ':orders-all'
    local member = enqueued and string.format('%013d|%s', enqueued, original) or nil
    if not member or not readable(all, 'zset') or not redis.call('ZSCORE', all, member) then
        redis.call('HDEL', receipt_meta, 'orders_all_complete_since_ms')
    end
    redis.call('HSET', okey, 'status', status)
    redis.call('ZREM', receipt_meta .. ':orders-due', original)
    local to = redis.call('HGET', okey, 'to')
    if to then
        local left = redis.call('HINCRBY', receipt_meta .. ':orders-unacted', to, -1)
        if left <= 0 then redis.call('HDEL', receipt_meta .. ':orders-unacted', to) end
    end
    return true
end

local ORDER_LATENCY_BOUNDS = {1000, 5000, 15000, 60000, 300000, 900000, 3600000}

-- Records `Acted` for the delivered-and-unacted messages of `queue_hex`, the machine
-- and session hex of a session queue joined by a colon (at
-- most `limit`), in the order they were delivered. Called inside the activity
-- event's own script, so the decision follows the relay server's execution
-- order of the Delivered receipt and the event, never a host clock.
receipt_acted = function(queue_hex, session, now, limit)
    local ukey = receipt_meta .. ':unacted:' .. queue_hex
    local rows = redis.call('ZRANGE', ukey, 0, limit - 1)
    for _, row in ipairs(rows) do
        redis.call('ZREM', ukey, row)
        local ok, m = pcall(cjson.decode, row)
        if ok and type(m) == 'table' and type(m.o) == 'string' then
            local fresh = receipt_emit({
                original = m.o, attempt = m.a, idpart = 'acted', outcome = 'acted',
                sender = m.s, target = m.t, now = now, session = session,
                expect = m.e, delivered_at = m.d,
            })
            if fresh and m.ord then
                local okey = order_key(m.o)
                if redis.call('EXISTS', okey) == 1 then
                    redis.call('HSET', okey, 'acted_at', now)
                    redis.call('ZREM', receipt_meta .. ':orders-expired', m.o)
                end
            end
            if fresh and m.ord and order_close(m.o, 'acted') then
                local ms, slot = now - m.d, #ORDER_LATENCY_BOUNDS
                for i, bound in ipairs(ORDER_LATENCY_BOUNDS) do
                    if ms <= bound then slot = i - 1; break end
                    slot = i
                end
                local lkey = receipt_meta .. ':order-latency:' .. string.match(queue_hex, '^(%x+):')
                redis.call('HINCRBY', lkey, 'b' .. slot, 1)
                redis.call('HINCRBY', lkey, 'count', 1)
                redis.call('HINCRBY', lkey, 'sum_ms', math.max(ms, 0))
            end
        end
    end
end
