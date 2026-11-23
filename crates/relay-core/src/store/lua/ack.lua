-- KEYS: inflight
-- ARGV: batch id, popper letter, optional delivery evidence, receipt keys
-- Deletes the rows of exactly that batch, popped by that popper (an id that another
-- consumer's batch holds is not touched). Returns how many it deleted, or -1 when
-- the in-flight key is of the wrong type (nothing is changed then).
if not readable(KEYS[1], 'list') or not receipt_ready() then return -1 end
local target = tonumber(ARGV[1])
local now = now_ms('')
local removed = 0
for _, row in ipairs(redis.call('LRANGE', KEYS[1], 0, -1)) do
    local id, who, deadline, entry = parse(row)
    if id == target and who == ARGV[2] then
        if ARGV[3] ~= '' then
            local observed
            entry, observed = receipt_observe(entry, now, ARGV[3])
            if observed then receipt_finish(entry, now, false) end
        end
        receipt_append(entry, 'acknowledged', now)
        receipt_finish(entry, now, true)
        redis.call('LREM', KEYS[1], 1, row)
        removed = removed + 1
    end
end
return removed
