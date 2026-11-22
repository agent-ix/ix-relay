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

local function string_end(json, start)
    local at = start + 1
    while at <= #json do
        local byte = string.byte(json, at)
        if byte == 92 then at = at + 2
        elseif byte == 34 then return at
        else at = at + 1 end
    end
    return nil
end

local function member_end(json, start)
    local at, depth = start, 0
    while at <= #json do
        local byte = string.byte(json, at)
        if byte == 34 then
            local last = string_end(json, at)
            if not last then return nil end
            at = last + 1
        elseif byte == 123 or byte == 91 then
            depth = depth + 1
            at = at + 1
        elseif byte == 125 or byte == 93 then
            if depth == 0 then return at end
            depth = depth - 1
            at = at + 1
        elseif byte == 44 and depth == 0 then return at
        else at = at + 1 end
    end
    return nil
end

-- Remove only top-level metadata spans. Strings, escaped quotes and nested
-- objects are scanned structurally; all other bytes, including integer lexemes,
-- remain untouched.
receipt_without_metadata = function(entry)
    local first = string.find(entry, '%S')
    if not first or string.byte(entry, first) ~= 123 then return entry end
    local at, separator, kept = first + 1, nil, 0
    local cuts = {}
    while at <= #entry do
        local key_start = string.find(entry, '%S', at)
        if not key_start or string.byte(entry, key_start) == 125 then break end
        if string.byte(entry, key_start) ~= 34 then return entry end
        local key_end = string_end(entry, key_start)
        if not key_end then return entry end
        local colon = string.find(entry, '%S', key_end + 1)
        if not colon or string.byte(entry, colon) ~= 58 then return entry end
        local ending = member_end(entry, colon + 1)
        if not ending then return entry end
        local valid, key = pcall(cjson.decode, string.sub(entry, key_start, key_end))
        if not valid then return entry end
        if key == '_relay' then
            local from = kept == 0 and key_start or separator
            local through = ending - 1
            if kept == 0 and string.byte(entry, ending) == 44 then through = ending end
            cuts[#cuts + 1] = {from, through}
        else
            kept = kept + 1
        end
        if string.byte(entry, ending) ~= 44 then break end
        separator, at = ending, ending + 1
    end
    if #cuts == 0 then return entry end
    local chunks, cursor = {}, 1
    for _, cut in ipairs(cuts) do
        if cut[1] > cursor then chunks[#chunks + 1] = string.sub(entry, cursor, cut[1] - 1) end
        cursor = math.max(cursor, cut[2] + 1)
    end
    chunks[#chunks + 1] = string.sub(entry, cursor)
    return table.concat(chunks)
end

-- Change private metadata without decoding/re-encoding recipient content.
receipt_entry = function(entry, metadata)
    local core, count = split_count(receipt_without_metadata(entry))
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
        sender = message.from, target = meta.recipient or message.to,
        event_time_ms = outcome == 'delivered' and cjson.null or now,
        observed_at_ms = now, evidence = evidence or cjson.null,
        reason = reason or cjson.null
    }
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

receipt_enqueue = function(entry, original, now, recipient)
    local marked = receipt_entry(entry, {original = original, recipient = recipient})
    receipt_append(marked, 'queued', now)
    return marked
end
