-- KEYS: agents, inbox, in-flight rows, dead letters (all of one registration)
-- ARGV: name, record, dead letter cap
-- Moves every queued entry of the inbox to the dead-letter list, byte for byte, and
-- then removes the registration, if it is still exactly the record the caller read
-- (FR-053). Does nothing while a batch of the lead is in flight: when its lease runs
-- out it returns to the inbox and a later call takes it.
-- Returns {status, moved}: status 1 done, 0 the registration changed or is gone,
-- 2 a batch is in flight, -1 a key is of the wrong type (nothing is changed then).
if not (readable(KEYS[1], 'hash') and readable(KEYS[2], 'list')
    and readable(KEYS[3], 'list') and readable(KEYS[4], 'list')) then
    return { -1, 0 }
end
if redis.call('HGET', KEYS[1], ARGV[1]) ~= ARGV[2] then return { 0, 0 } end
if redis.call('LLEN', KEYS[3]) > 0 then return { 2, 0 } end
local items = redis.call('LRANGE', KEYS[2], 0, -1)
for _, entry in ipairs(items) do redis.call('RPUSH', KEYS[4], entry) end
if #items > 0 then
    redis.call('LTRIM', KEYS[2], #items, -1)
    local over = redis.call('LLEN', KEYS[4]) - tonumber(ARGV[3])
    if over > 0 then redis.call('LTRIM', KEYS[4], -tonumber(ARGV[3]), -1) end
end
redis.call('HDEL', KEYS[1], ARGV[1])
return { 1, #items }
