-- KEYS: inflight. ARGV: batch, popper, evidence, now, receipt keys.
if not readable(KEYS[1], 'list') or not receipt_ready() then return -1 end
local target, now, count = tonumber(ARGV[1]), now_ms(ARGV[4]), 0
local rows = redis.call('LRANGE', KEYS[1], 0, -1)
for index, row in ipairs(rows) do
    local id, who, deadline, entry = parse(row)
    if id == target and who == ARGV[2] then
        local marked, observed = receipt_observe(entry, now, ARGV[3])
        if observed then
            redis.call('LSET', KEYS[1], index - 1, make_row(id, who, deadline, marked))
            receipt_finish(entry, now, false)
            count = count + 1
        end
    end
end
return count
