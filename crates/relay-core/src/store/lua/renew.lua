-- KEYS: inflight
-- ARGV: batch id, now override, lease ms, max lease ms, popper letter
-- Extends the lease of the rows of exactly that batch, popped by that popper (an id that
-- another consumer's batch now holds is not touched), to now + lease, never
-- shortening one. For a consumer that knows the batch is being held elsewhere and
-- must not be returned to the inbox or popped by another consumer meanwhile. A batch
-- that is no longer in flight (acknowledged, reaped) matches nothing and nothing is
-- changed. Returns how many rows it extended, or -1 when the key is of the wrong
-- type (nothing is changed then).
if not readable(KEYS[1], 'list') then return -1 end
local target = tonumber(ARGV[1])
local lease = math.min(tonumber(ARGV[3]), tonumber(ARGV[4]))
local until_ms = now_ms(ARGV[2]) + lease
local rows = redis.call('LRANGE', KEYS[1], 0, -1)
local extended = 0
for i, row in ipairs(rows) do
    local id, who, deadline, entry = parse(row)
    if id == target and who == ARGV[5] and deadline < until_ms then
        redis.call('LSET', KEYS[1], i - 1, make_row(id, who, until_ms, entry))
        extended = extended + 1
    end
end
return extended
