-- One hook activity event, written in one request.
-- KEYS[1] agents hash, KEYS[2] activity stream
-- ARGV[1] lead name, ARGV[2] kind, ARGV[3] event object (JSON, at least one
-- field), ARGV[4] last-activity key stem, ARGV[5] max events, ARGV[6] max age
-- ms, ARGV[7] last-activity TTL seconds
-- Returns 1 when written, 0 when the lead has no registered lane address.
local raw = redis.call('HGET', KEYS[1], ARGV[1])
if not raw then return 0 end
local ok, reg = pcall(cjson.decode, raw)
if not ok or type(reg) ~= 'table' or type(reg.machine) ~= 'string'
  or type(reg.place) ~= 'table' or type(reg.place.workspace) ~= 'string'
  or type(reg.place.tab) ~= 'string' then return 0 end
local addr = reg.machine .. ':' .. reg.place.workspace .. ':' .. reg.place.tab
if string.len(addr) > 256 then return 0 end
local t = redis.call('TIME')
local ms = tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
local event = '{"addr":' .. cjson.encode(addr) .. ',"server_ms":' .. string.format('%d', ms)
  .. ',' .. string.sub(ARGV[3], 2)
redis.call('XADD', KEYS[2], 'MAXLEN', '=', ARGV[5], '*', 'event', event)
local floor = math.max(0, ms - tonumber(ARGV[6]))
redis.call('XTRIM', KEYS[2], 'MINID', '=', string.format('%d', floor) .. '-0')
local last = ARGV[4] .. addr
redis.call('HSET', last, 'kind:' .. ARGV[2], event, 'any', event)
redis.call('EXPIRE', last, ARGV[7])
return 1
