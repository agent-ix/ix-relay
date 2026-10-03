-- KEYS: pending inbox list
-- ARGV: maximum entries to return
-- Returns {status, pending count, oldest entries}; this script is read-only.
local kind = redis.call('TYPE', KEYS[1]).ok
if kind ~= 'none' and kind ~= 'list' then
    return { -1, 0, {} }
end
local pending = redis.call('LLEN', KEYS[1])
local entries = redis.call('LRANGE', KEYS[1], 0, tonumber(ARGV[1]) - 1)
return { 0, pending, entries }
