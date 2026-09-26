-- KEYS: inbox, inflight, dead letters, batch id counter
-- ARGV: max count, max bytes, weight slack, popper letter, lease ms, now override,
--       max redeliveries, dead letter cap, max in-flight batches, max in-flight bytes,
--       in-flight row overhead
-- Returns {status, pending, batch id (0 for none), {entries}, {redelivery counts},
--          stuck, evicted}. Status 0 took what it returned (maybe nothing: an empty
-- inbox), 1 took nothing because the in-flight caps would be exceeded, -1 took
-- nothing because a key of this lead is of the wrong type.
-- The counter must be absent or an integer `INCR` accepts: checked before any write.
local function unreadable()
    if not (readable(KEYS[1], 'list') and readable(KEYS[2], 'list') and readable(KEYS[4], 'string')) then
        return true
    end
    local counter = redis.call('GET', KEYS[4])
    return counter and (#counter > 17 or string.find(counter, '%D')) and true or false
end
if unreadable() then return { -1, 0, 0, {}, {}, 0, 0 } end
local now = now_ms(ARGV[6])
local reaped = reap(KEYS[1], KEYS[2], KEYS[3], now, tonumber(ARGV[7]), tonumber(ARGV[8]))
local stuck, evicted = reaped[3], reaped[4]

-- Nothing waiting: nothing to take and no cap to report.
if redis.call('LLEN', KEYS[1]) == 0 then return { 0, 0, 0, {}, {}, stuck, evicted } end

local batches, bytes, seen = 0, 0, {}
for _, row in ipairs(redis.call('LRANGE', KEYS[2], 0, -1)) do
    if not is_stuck(row, now, tonumber(ARGV[7])) then
        local id = parse(row)
        if not seen[id] then
            seen[id] = true
            batches = batches + 1
        end
        bytes = bytes + #row
    end
end
if batches + 1 > tonumber(ARGV[9]) then
    return { 1, redis.call('LLEN', KEYS[1]), 0, {}, {}, stuck, evicted }
end

local items = redis.call('LRANGE', KEYS[1], 0, tonumber(ARGV[1]) - 1)
local budget = tonumber(ARGV[2])
local slack = tonumber(ARGV[3])
local function count(s, pattern)
    local _, n = string.gsub(s, pattern, '')
    return n
end
local total, n, stored = 0, 0, 0
for i, v in ipairs(items) do
    local w = #v + slack
        + 3 * count(v, '\\n')
        + count(v, '\\[rbf]')
        + 2 * count(v, '[%z\1-\31\127]')
        + count(v, '\194[\128-\159]')
        + count(v, '\216\156')
    if i > 1 and total + w > budget then break end
    total = total + w
    stored = stored + #v + tonumber(ARGV[11])
    n = i
end
if n == 0 then return { 0, 0, 0, {}, {}, stuck, evicted } end
-- Like the first message of a batch, a single entry is always taken when nothing is
-- in flight, however large: the cap only ever holds back a pop behind other rows.
if bytes > 0 and bytes + stored > tonumber(ARGV[10]) then
    return { 1, redis.call('LLEN', KEYS[1]), 0, {}, {}, stuck, evicted }
end

-- Destination first (the in-flight rows), the source (the inbox) last.
local id = redis.call('INCR', KEYS[4])
local deadline = now + tonumber(ARGV[5])
local taken, counts = {}, {}
for i = 1, n do
    local _, c = split_count(items[i])
    redis.call('RPUSH', KEYS[2], make_row(id, ARGV[4], deadline, items[i]))
    taken[i] = items[i]
    counts[i] = c
end
redis.call('LTRIM', KEYS[1], n, -1)
return { 0, redis.call('LLEN', KEYS[1]), id, taken, counts, stuck, evicted }
