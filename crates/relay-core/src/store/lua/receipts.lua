-- Receipt keys are the final two ARGV values of an instrumented script.
local receipt_stream, receipt_meta = ARGV[#ARGV - 1], ARGV[#ARGV]

receipt_ready = function()
    return readable(receipt_stream, 'stream') and readable(receipt_meta, 'hash')
end

local function receipt_message(entry)
    local ok, message = pcall(cjson.decode, entry)
    if ok and type(message) == 'table' and type(message.from) == 'string'
        and (type(message.to) == 'string') then return message end
    return nil
end

receipt_metadata = function(entry)
    local message = receipt_message(entry)
    if not message or type(message._relay) ~= 'table' then return nil end
    local meta = message._relay
    if type(meta.original) ~= 'string' then return nil end
    return meta
end

-- Change private metadata without decoding/re-encoding recipient content.
receipt_entry = function(entry, metadata)
    local core, count = split_count(entry)
    core = string.gsub(core, ',"_relay":%{[^{}]*%}', '')
    if string.sub(core, -1) ~= '}' then return entry end
    return string.sub(core, 1, -2) .. ',"_relay":' .. cjson.encode(metadata) .. '}'
        .. string.rep(' ', count)
end

receipt_gap = function(now)
    local first = tonumber(redis.call('HGET', receipt_meta, 'gap_start'))
    local last = tonumber(redis.call('HGET', receipt_meta, 'gap_end'))
    redis.call('HSET', receipt_meta, 'gap_start', math.min(first or now, now),
        'gap_end', math.max(last or now, now))
end

receipt_append = function(entry, outcome, now, evidence, reason)
    local message = receipt_message(entry)
    local meta = receipt_metadata(entry)
    if not meta or not message then receipt_gap(now); return true end
    redis.call('HSETNX', receipt_meta, 'instrumentation_start_ms', now)
    local attempt = meta.attempt or cjson.null
    local event_id = meta.original .. '/' .. (meta.attempt or 'enqueue') .. '/' .. outcome
    if redis.call('HEXISTS', receipt_meta, 'event:' .. event_id) == 1 then return false end
    local event = {
        event_id = event_id,
        original_id = meta.original, attempt_id = attempt, outcome = outcome,
        sender = message.from, target = message.to,
        event_time_ms = outcome == 'delivered' and cjson.null or now,
        observed_at_ms = now, evidence = evidence or cjson.null,
        reason = reason or cjson.null
    }
    local cursor = redis.call('XADD', receipt_stream, '*', 'event', cjson.encode(event))
    redis.call('HSET', receipt_meta, 'event:' .. event_id, cursor)
    local count = redis.call('XLEN', receipt_stream)
    if count > 10000 then
        local evicted = redis.call('XRANGE', receipt_stream, '-', '+', 'COUNT', count - 10000)
        for _, row in ipairs(evicted) do
            local old = cjson.decode(row[2][2])
            redis.call('HDEL', receipt_meta, 'event:' .. old.event_id)
            local boundary = tonumber(redis.call('HGET', receipt_meta, 'evicted_through_ms'))
            redis.call('HSET', receipt_meta, 'evicted_through_ms',
                math.max(boundary or old.observed_at_ms, old.observed_at_ms),
                'evicted_through_cursor', row[1])
        end
        redis.call('XTRIM', receipt_stream, 'MAXLEN', 10000)
    end
    return true
end

receipt_pending = function(entry, now)
    local meta = receipt_metadata(entry)
    if meta and meta.attempt then
        redis.call('HSET', receipt_meta, 'pending:' .. meta.original .. '/' .. meta.attempt, now)
    end
end

receipt_finish = function(entry, now, uncertain)
    local meta = receipt_metadata(entry)
    if meta and meta.attempt then
        local key = 'pending:' .. meta.original .. '/' .. meta.attempt
        local pending = redis.call('HGET', receipt_meta, key)
        if pending and uncertain then
            receipt_gap(tonumber(pending))
            receipt_gap(now)
        end
        redis.call('HDEL', receipt_meta, key)
    end
end

receipt_enqueue = function(entry, original, now)
    local marked = receipt_entry(entry, {original = original})
    receipt_append(marked, 'queued', now)
    return marked
end
