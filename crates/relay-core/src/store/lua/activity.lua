-- One hook activity event, written in one request.
-- KEYS[1] activity stream
-- ARGV[1] lane address (verified by the caller), ARGV[2] kind, ARGV[3] event
-- object (JSON, at least one field), ARGV[4] last-activity key stem,
-- ARGV[5] max events, ARGV[6] max age ms, ARGV[7] last-activity TTL seconds
local t = redis.call('TIME')
local ms = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
local event = '{"addr":' .. cjson.encode(ARGV[1]) .. ',"server_ms":' .. string.format('%d', ms)
  .. ',' .. string.sub(ARGV[3], 2)
redis.call('XADD', KEYS[1], 'MAXLEN', '=', ARGV[5], '*', 'event', event)
local floor = math.max(0, ms - tonumber(ARGV[6]))
redis.call('XTRIM', KEYS[1], 'MINID', '=', string.format('%d', floor) .. '-0')
local last = ARGV[4] .. ARGV[1]
redis.call('HSET', last, 'kind:' .. ARGV[2], event, 'any', event)
redis.call('EXPIRE', last, ARGV[7])
return 1
