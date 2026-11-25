-- Shared by the assignment scripts: the transition record and its capped stream.
local function assign_ids(entries)
    local ids = {}
    for _, entry in ipairs(entries) do
        ids[#ids + 1] = cjson.encode(cjson.decode(entry).id)
    end
    return '[' .. table.concat(ids, ',') .. ']'
end

-- Appends one transition to the stream, keeps its count and age bounds and renews its
-- inactivity expiry, as the work history does (FR-066).
local function assign_log(events, start, tickets, op, lane, writer, order, from, reason, version, ms,
        maxlen, max_age, ttl)
    local record = '{"op":' .. cjson.encode(op) .. ',"lane":' .. cjson.encode(lane)
        .. ',"writer":' .. cjson.encode(writer) .. ',"version":' .. string.format('%d', version)
        .. ',"tickets":' .. tickets
    if order ~= '' then record = record .. ',"order_id":' .. cjson.encode(order) end
    if from ~= '' then record = record .. ',"from":' .. cjson.encode(from) end
    if reason ~= '' then record = record .. ',"reason":' .. cjson.encode(reason) end
    record = record .. ',"at_ms":' .. string.format('%d', ms) .. '}'
    redis.call('XADD', events, 'MAXLEN', '=', maxlen, '*', 'record', record)
    redis.call('XTRIM', events, 'MINID', '=', string.format('%d', math.max(0, ms - max_age)) .. '-0')
    redis.call('SET', start, string.format('%d', ms), 'NX')
    redis.call('EXPIRE', events, ttl)
    redis.call('EXPIRE', start, ttl)
end

-- A write to `lane` settles the defects whose lane address is `lane`: the list now under that
-- address is the one the orchestrator chose. The defect of the other lane of a swap still
-- stands.
local function assign_settle(defects, lane)
    local all = redis.call('HGETALL', defects)
    for i = 1, #all, 2 do
        if cjson.decode(all[i + 1]).to == lane then redis.call('HDEL', defects, all[i]) end
    end
end

-- A move that could not happen: a transition naming both addresses and the reason, and a
-- defect that stands until a write to the lane at `to` or a successful move of that pair.
local function assign_move_failed(keys, from, to, writer, order, reason, ms, maxlen, max_age, ttl)
    local version = tonumber(redis.call('HGET', keys.meta, 'version')) or 0
    assign_log(keys.events, keys.start, assign_ids(redis.call('LRANGE', keys.list, 0, -1)),
        'move_failed', to, writer, order, from, reason, version, ms, maxlen, max_age, ttl)
    local defect = '{"from":' .. cjson.encode(from) .. ',"to":' .. cjson.encode(to)
        .. ',"reason":' .. cjson.encode(reason) .. ',"at_ms":' .. string.format('%d', ms) .. '}'
    redis.call('HSET', keys.defects, from .. '>' .. to, defect)
end
