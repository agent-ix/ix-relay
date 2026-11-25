-- KEYS: inbox. ARGV: max count, receipt keys.
if not readable(KEYS[1], 'list') or not receipt_ready() then
    return redis.error_reply('inbox or receipt history has wrong type')
end
local now = now_ms('')
local taken = redis.call('LRANGE', KEYS[1], 0, tonumber(ARGV[1]) - 1)
for i, entry in ipairs(taken) do
    local metadata = receipt_metadata(entry)
    if metadata then
        metadata.attempt = 'plain/' .. redis.call('HINCRBY', receipt_meta, 'plain_sequence', 1)
        metadata.delivered = nil
        taken[i] = receipt_entry(entry, metadata)
        -- The recipient took it: Delivered, so the order can be Acted or expire.
        receipt_observe(taken[i], now, 'inbox_read')
        receipt_append(taken[i], 'removed', now, nil, 'plain_inbox_pop')
    else
        receipt_gap(now)
    end
end
if #taken > 0 then redis.call('LTRIM', KEYS[1], #taken, -1) end
return taken
