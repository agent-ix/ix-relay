-- KEYS: agents, inbox of the replaced registration, inbox of the one holding its address
-- ARGV: replaced name, replaced record, holder name, holder record
-- Moves every queued entry of the first inbox to the front of the second, in order,
-- each marked as forwarded from a prior session of the address (FR-053), if both
-- registrations are still exactly the records the caller read. The mark is spliced
-- into the entry's JSON text, not decoded and re-encoded, so no other member
-- changes; an entry that already has it (a second move) keeps it, and an entry that
-- is not a JSON object is moved as it is. Redelivery spaces travel with the entry.
-- Returns {status, moved}: status 1 done (moved may be 0), 0 a registration changed
-- or is gone, -1 a key is of the wrong type (nothing is changed then).
if not (readable(KEYS[1], 'hash') and readable(KEYS[2], 'list') and readable(KEYS[3], 'list')) then
    return { -1, 0 }
end
if redis.call('HGET', KEYS[1], ARGV[1]) ~= ARGV[2]
    or redis.call('HGET', KEYS[1], ARGV[3]) ~= ARGV[4] then
    return { 0, 0 }
end
local items = redis.call('LRANGE', KEYS[2], 0, -1)
if #items == 0 then return { 1, 0 } end
local mark = ',"forwarded":true'
local marked = {}
for i, entry in ipairs(items) do
    local core, n = split_count(entry)
    if string.sub(core, -1) == '}' and not string.find(core, mark, 1, true) then
        core = string.sub(core, 1, -2) .. mark .. '}'
    end
    marked[i] = core .. string.rep(' ', n)
end
-- Destination first, source last: a failure part-way leaves a duplicate, never a loss.
for i = #marked, 1, -1 do redis.call('LPUSH', KEYS[3], marked[i]) end
redis.call('LTRIM', KEYS[2], #items, -1)
return { 1, #items }
