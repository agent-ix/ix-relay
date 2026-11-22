-- KEYS: inbox, ordinary log. ARGV: lead, log cap, stored entries, receipt keys.
if not readable(KEYS[1], 'list') or not readable(KEYS[2], 'stream') or not receipt_ready() then
    return redis.error_reply('inbox or history has wrong type')
end
local now = now_ms('')
for i = #ARGV - 2, 3, -1 do
    if receipt_append(ARGV[i], 'requeued', now, nil, 'plain_inbox_requeue') then
        redis.call('LPUSH', KEYS[1], ARGV[i])
    end
end
redis.call('XADD', KEYS[2], 'MAXLEN', '~', ARGV[2], '*', 'requeued', ARGV[1])
return 1
