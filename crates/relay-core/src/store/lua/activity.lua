-- One hook activity event, written in one request.
-- KEYS[1] activity stream
-- ARGV[1] lane address (verified by the caller), ARGV[2] kind, ARGV[3] event
-- object (JSON, at least one field), ARGV[4] last-activity key stem,
-- ARGV[5] max events, ARGV[6] max age ms, ARGV[7] last-activity TTL seconds,
-- ARGV[8] hex of the lane's machine and of the event's session joined by a colon
-- (empty: none), ARGV[9] the session, ARGV[10] 1 when this event may record Acted,
-- then the receipt stream and metadata keys.
local ms = now_ms('')
local event = '{"addr":' .. cjson.encode(ARGV[1]) .. ',"server_ms":' .. string.format('%d', ms)
  .. ',' .. string.sub(ARGV[3], 2)
redis.call('XADD', KEYS[1], 'MAXLEN', '=', ARGV[5], '*', 'event', event)
local floor = math.max(0, ms - tonumber(ARGV[6]))
redis.call('XTRIM', KEYS[1], 'MINID', '=', string.format('%d', floor) .. '-0')
local last = ARGV[4] .. ARGV[1]
redis.call('HSET', last, 'kind:' .. ARGV[2], event, 'any', event)
redis.call('EXPIRE', last, ARGV[7])
-- A prompt or tool event of the session that follows a Delivered receipt records
-- Acted (FR-067), in the same request as the event.
local acts = ARGV[2] == 'prompt' or ARGV[2] == 'pre_tool' or ARGV[2] == 'post_tool'
if acts and ARGV[10] == '1' and ARGV[8] ~= '' and receipt_ready() then
    receipt_acted(ARGV[8], ARGV[9], ms, 50)
end
return 1
