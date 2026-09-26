-- KEYS: inflight
-- ARGV: batch id, popper letter
-- Deletes the rows of exactly that batch, popped by that popper (an id that another
-- consumer's batch holds is not touched). Returns how many it deleted, or -1 when
-- the in-flight key is of the wrong type (nothing is changed then).
if not readable(KEYS[1], 'list') then return -1 end
local target = tonumber(ARGV[1])
local removed = 0
for _, row in ipairs(redis.call('LRANGE', KEYS[1], 0, -1)) do
    local id, who = parse(row)
    if id == target and who == ARGV[2] then
        redis.call('LREM', KEYS[1], 1, row)
        removed = removed + 1
    end
end
return removed
