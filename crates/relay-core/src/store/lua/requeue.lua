-- KEYS: inflight, inbox, log
-- ARGV: batch id, lead name, log cap, popper letter
-- Moves the rows of exactly that batch, popped by that popper, back to the front of the inbox in their
-- original order, entries (and so redelivery counts) as they were, and wakes a
-- watcher on the log. Returns how many it moved, or -1 when a key is of the wrong
-- type (nothing is changed then).
if not (readable(KEYS[1], 'list') and readable(KEYS[2], 'list') and readable(KEYS[3], 'stream')) then
    return -1
end
local target = tonumber(ARGV[1])
local back, rows = {}, {}
for _, row in ipairs(redis.call('LRANGE', KEYS[1], 0, -1)) do
    local id, who, deadline, entry = parse(row)
    if id == target and who == ARGV[4] then
        back[#back + 1] = entry
        rows[#rows + 1] = row
    end
end
if #back == 0 then return 0 end
for i = #back, 1, -1 do redis.call('LPUSH', KEYS[2], back[i]) end
redis.call('XADD', KEYS[3], 'MAXLEN', '~', ARGV[3], '*', 'requeued', ARGV[2])
for _, row in ipairs(rows) do redis.call('LREM', KEYS[1], 1, row) end
return #back
