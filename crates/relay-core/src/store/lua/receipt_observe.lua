-- KEYS: inflight. ARGV: batch, popper, evidence, now, receipt keys.
if not readable(KEYS[1], 'list') or not receipt_ready() then return -1 end
local target, now, count = tonumber(ARGV[1]), now_ms(ARGV[4]), 0
local rows = redis.call('LRANGE', KEYS[1], 0, -1)
for index, row in ipairs(rows) do
    local id, who, deadline, entry = parse(row)
    if id == target and who == ARGV[2] then
        local metadata = receipt_metadata(entry)
        if metadata and not metadata.delivered then
            receipt_append(entry, 'delivered', now, ARGV[3])
            metadata.delivered = ARGV[3]
            redis.call('LSET', KEYS[1], index - 1, make_row(id, who, deadline, receipt_entry(entry, metadata)))
            receipt_finish(entry, now, false)
            count = count + 1
        end
    end
end
return count
