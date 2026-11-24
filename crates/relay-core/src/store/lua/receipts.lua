
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


receipt_append = function(entry, outcome, now, evidence, reason)
    local message = receipt_message(entry)
    local meta = receipt_metadata(entry)
    if not meta or not message then receipt_gap(now); return true end
    return receipt_emit({
        original = meta.original, attempt = meta.attempt,
        idpart = meta.attempt or 'enqueue', outcome = outcome,
        sender = message.from, target = meta.recipient or message.to,
        now = now, evidence = evidence, reason = reason, expect = meta.expect,
    })
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

-- Shared by observation-only and combined observation/settlement operations.
-- Return the marked entry so a caller can retain it or settle it in the same script.
receipt_observe = function(entry, now, evidence)
    local metadata = receipt_metadata(entry)
    if not metadata or metadata.delivered then return entry, false end
    receipt_append(entry, 'delivered', now, evidence)
    metadata.delivered = evidence
    local message = receipt_message(entry)
    local machine, session = nil, nil
    if type(metadata.recipient) == 'string' then
        machine, session = string.match(metadata.recipient, '^session/(%x+)/(%x+)$')
    end
    if session and message then
        local ukey = receipt_meta .. ':unacted:' .. machine .. ':' .. session
        redis.call('ZADD', ukey, now, cjson.encode({
            o = metadata.original, a = metadata.attempt, s = message.from,
            t = metadata.recipient, e = metadata.expect, d = now,
            ord = metadata.order and true or nil,
        }))
        redis.call('ZREMRANGEBYRANK', ukey, 0, -1001)
        redis.call('EXPIRE', ukey, 604800)
    end
    if metadata.order then
        local okey = order_key(metadata.original)
        -- The deadline runs from the first Delivered; a redelivery or a lease
        -- timeout does not move it.
        if redis.call('HGET', okey, 'status') == 'open'
            and redis.call('HSETNX', okey, 'delivered_at', now) == 1 then
            redis.call('ZADD', receipt_meta .. ':orders-due', now + metadata.order,
                metadata.original)
        end
    end
    return receipt_entry(entry, metadata), true
end

-- `order_ms` is the ack deadline of an order, or nil for a plain message.
receipt_enqueue = function(entry, original, now, recipient, order_ms, expect)
    local meta = {original = original, recipient = recipient}
    if order_ms then
        meta.order = order_ms
        if expect and expect ~= '' then meta.expect = expect end
    end
    local marked = receipt_entry(entry, meta)
    receipt_append(marked, 'queued', now)
    if order_ms then
        local message = receipt_message(entry)
        local okey = order_key(original)
        redis.call('HSET', okey, 'from', message.from, 'to', message.to,
            'recipient', recipient, 'ack_ms', order_ms, 'enqueued_at', now,
            'state', 'queued', 'status', 'open', 'expect', meta.expect or '')
        redis.call('EXPIRE', okey, 691200)
        local due = receipt_meta .. ':orders-due'
        redis.call('ZADD', due, now + order_ms, original)
        redis.call('HINCRBY', receipt_meta .. ':orders-unacted', message.to, 1)
        local over = redis.call('ZCARD', due) - 10000
        if over > 0 then
            for _, old in ipairs(redis.call('ZRANGE', due, 0, over - 1)) do
                order_close(old, 'evicted')
            end
        end
    end
    return marked
end
