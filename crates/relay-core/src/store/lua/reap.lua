-- KEYS: three per lead, in this order: inbox, inflight, dead letters
-- ARGV: now override, max redeliveries, dead letter cap
-- Returns, per lead, {requeued, dead-lettered, stuck, evicted}, or {-1, 0, 0, 0} for
-- a lead a key of which is of the wrong type: nothing of it is changed, and one
-- bad lead never stops the rest.
local now = now_ms(ARGV[1])
local out = {}
for i = 0, #KEYS / 3 - 1 do
    local inbox, inflight, dead = KEYS[3 * i + 1], KEYS[3 * i + 2], KEYS[3 * i + 3]
    if readable(inbox, 'list') and readable(inflight, 'list') then
        out[#out + 1] = reap(inbox, inflight, dead, now, tonumber(ARGV[2]), tonumber(ARGV[3]))
    else
        out[#out + 1] = { -1, 0, 0, 0 }
    end
end
return out
