-- KEYS: inflight, dead letters
-- ARGV: batch id, dead letter cap, popper letter
-- Moves the rows of exactly that batch, popped by that popper (a batch of another
-- consumer that holds the id is not touched), from the in-flight list to the dead-letter
-- list, entries as they were (without their redelivery spaces), and trims the
-- dead-letter list to its cap. For a consumer that knows the batch will never be
-- delivered (the user denied it). Returns how many it moved, or -1 when a key is
-- of the wrong type (nothing is changed then).
if not (readable(KEYS[1], 'list') and readable(KEYS[2], 'list')) then return -1 end
local target = tonumber(ARGV[1])
local buried, rows = {}, {}
for _, row in ipairs(redis.call('LRANGE', KEYS[1], 0, -1)) do
    local id, who, deadline, entry = parse(row)
    if id == target and who == ARGV[3] then
        local core = split_count(entry)
        buried[#buried + 1] = core
        rows[#rows + 1] = row
    end
end
if #buried == 0 then return 0 end
for _, entry in ipairs(buried) do redis.call('RPUSH', KEYS[2], entry) end
for _, row in ipairs(rows) do redis.call('LREM', KEYS[1], 1, row) end
local over = redis.call('LLEN', KEYS[2]) - tonumber(ARGV[2])
if over > 0 then redis.call('LTRIM', KEYS[2], -tonumber(ARGV[2]), -1) end
return #buried
