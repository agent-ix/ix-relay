-- One assignment-list write with its transition record, in one request.
-- KEYS: 1 list, 2 meta, 3 lanes set, 4 events stream, 5 events start, 6 defects hash,
-- 7 move defects hash.
-- ARGV: 1 op (set, append, remove, reorder or stale), 2 lane, 3 writer, 4 order id (empty:
-- none), 5 max tickets, 6 max lanes, 7 max events, 8 max age ms, 9 events ttl seconds,
-- 10 the lane's host, then the payload: entries (JSON) for set and append, else ticket ids.
-- Replies {status, version, written ms, entries}: status 1 written, with the list as it now
-- stands; -1 too many tickets; -2 stale host (defect raised, nothing else written); -3
-- duplicate and -4 no such ticket (entries holds the id); -5 reorder mismatch; -6 lane limit;
-- -7 wrong key type (nothing written).
local function refuse(status, id)
    return {status, 0, 0, {id or ''}}
end
local ms = now_ms('')
local op = ARGV[1]
if not (readable(KEYS[1], 'list') and readable(KEYS[2], 'hash') and readable(KEYS[3], 'set')
    and readable(KEYS[4], 'stream') and readable(KEYS[5], 'string') and readable(KEYS[6], 'hash')
    and readable(KEYS[7], 'hash')) then
    return refuse(-7)
end
if op == 'stale' then
    redis.call('HSET', KEYS[6], ARGV[10], string.format('%d', ms))
    redis.call('HINCRBY', KEYS[6], 'count:' .. ARGV[10], 1)
    redis.call('EXPIRE', KEYS[6], ARGV[9])
    return refuse(-2)
end
local max = tonumber(ARGV[5])
local current = redis.call('LRANGE', KEYS[1], 0, -1)
if redis.call('SISMEMBER', KEYS[3], ARGV[2]) == 0 and redis.call('SCARD', KEYS[3]) >= tonumber(ARGV[6]) then
    return refuse(-6)
end
local held = {}
for i, entry in ipairs(current) do held[cjson.decode(entry).id] = i end
local payload = {}
for i = 11, #ARGV do payload[#payload + 1] = ARGV[i] end
if op == 'set' then
    if #payload > max then return refuse(-1) end
    redis.call('DEL', KEYS[1])
    if #payload > 0 then redis.call('RPUSH', KEYS[1], unpack(payload)) end
    current = payload
elseif op == 'append' then
    if #current + #payload > max then return refuse(-1) end
    for _, entry in ipairs(payload) do
        local id = cjson.decode(entry).id
        if held[id] then return refuse(-3, id) end
        held[id] = true
    end
    if #payload > 0 then redis.call('RPUSH', KEYS[1], unpack(payload)) end
    for _, entry in ipairs(payload) do current[#current + 1] = entry end
elseif op == 'remove' then
    local at = held[payload[1]]
    if not at then return refuse(-4, payload[1]) end
    redis.call('LREM', KEYS[1], 1, current[at])
    table.remove(current, at)
else
    if #payload ~= #current then return refuse(-5) end
    local ordered = {}
    for _, id in ipairs(payload) do
        local at = held[id]
        if not at then return refuse(-5) end
        ordered[#ordered + 1] = current[at]
        held[id] = nil
    end
    redis.call('DEL', KEYS[1])
    if #ordered > 0 then redis.call('RPUSH', KEYS[1], unpack(ordered)) end
    current = ordered
end
local version = redis.call('HINCRBY', KEYS[2], 'version', 1)
redis.call('HSET', KEYS[2], 'at_ms', string.format('%d', ms))
-- A lane counts toward the lane limit only while it holds tickets.
if #current > 0 then
    redis.call('SADD', KEYS[3], ARGV[2])
else
    redis.call('SREM', KEYS[3], ARGV[2])
end
assign_settle(KEYS[7], ARGV[2])
assign_log(KEYS[4], KEYS[5], assign_ids(current), op, ARGV[2], ARGV[3], ARGV[4], '', '', version, ms,
    ARGV[7], tonumber(ARGV[8]), ARGV[9])
return {1, version, ms, current}
