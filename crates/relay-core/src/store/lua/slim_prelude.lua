-- What the scripts that write only receipt events need of common.lua: the
-- forward declarations of receipt_core.lua, the clock and the type check.
local receipt_ready, receipt_gap, receipt_emit, order_close, receipt_acted

local function now_ms(override)
    if override ~= '' then return tonumber(override) end
    local t = redis.call('TIME')
    return tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
end

local function readable(key, want)
    local kind = redis.call('TYPE', key)['ok']
    return kind == 'none' or kind == want
end
