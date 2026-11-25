-- KEYS: publication fences, recipient inboxes, ordinary message log.
-- ARGV: fence count, recipient count, expected publications, message, log cap,
--       optional clock override, order ack ms (empty: plain), expected ticket,
--       recipient names, order retention ms, receipt stream, receipt metadata.
local fences, recipients = tonumber(ARGV[1]), tonumber(ARGV[2])
for i = 1, fences do
    if redis.call('GET', KEYS[i]) ~= ARGV[i + 2] then return 0 end
end
for i = fences + 1, fences + recipients do
    if not readable(KEYS[i], 'list') then return redis.error_reply('inbox has wrong type') end
end
local log = KEYS[fences + recipients + 1]
if not readable(log, 'stream') or not receipt_ready() then
    return redis.error_reply('message or receipt history has wrong type')
end
if ARGV[fences + 6] ~= ''
    and not readable(receipt_meta .. ':orders-all', 'zset') then
    return redis.error_reply('order history has wrong type')
end
local now = now_ms(ARGV[fences + 5])
local entry = ARGV[fences + 3]
local id = redis.call('XADD', log, 'MAXLEN', '~', ARGV[fences + 4], '*', 'msg', entry)
for i = 1, recipients do
    local marked = receipt_enqueue(entry, id .. '/' .. i, now, ARGV[fences + 7 + i],
        tonumber(ARGV[fences + 6]), ARGV[fences + 7],
        tonumber(ARGV[fences + 8 + recipients]))
    redis.call('RPUSH', KEYS[fences + i], marked)
end
return 1
