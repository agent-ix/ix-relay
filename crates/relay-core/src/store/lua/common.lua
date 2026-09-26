-- Shared by every in-flight script (see `store/inflight.rs`).
--
-- One in-flight row is one popped entry: `<batch id> <popper letter> <deadline ms> <entry>`,
-- the entry byte for byte after the third space. An entry that was redelivered n
-- times carries n trailing spaces (a stored `Message` is JSON and never ends in
-- one), so its count travels with it through the inbox, a requeue and a re-pop;
-- an identical fresh entry has none.
--
-- WRITE DISCIPLINE. Redis does not roll back what a script wrote before it
-- errored. So every script first checks the type of every key it may write
-- (`readable`) and refuses without writing anything, and then writes the
-- destination of each move before it deletes the source.

local function now_ms(override)
    if override ~= '' then return tonumber(override) end
    local t = redis.call('TIME')
    return tonumber(t[1]) * 1000 + math.floor(tonumber(t[2]) / 1000)
end

local function parse(row)
    local id, who, deadline, from = string.match(row, '^(%d+) (%a) (%d+) ()')
    if not id then return nil end
    return tonumber(id), who, tonumber(deadline), string.sub(row, from)
end

local function make_row(id, who, deadline, entry)
    return string.format('%d %s %d ', id, who, deadline) .. entry
end

-- An entry without its redelivery spaces, and how many it had.
local function split_count(entry)
    local core = string.gsub(entry, ' +$', '')
    return core, #entry - #core
end

-- Whether a row is one the reap could not move: due, but out of redeliveries (or
-- unparseable) with a dead-letter key that cannot take it. Such rows are not
-- counted against the in-flight caps, or a broken dead-letter key would stop
-- every pop once enough of them piled up.
local function is_stuck(row, now, max_redeliveries)
    local id, who, deadline, entry = parse(row)
    if not id then return true end
    if deadline > now then return false end
    local _, n = split_count(entry)
    return n >= max_redeliveries
end

-- Whether `key` is absent or of type `want`.
local function readable(key, want)
    local kind = redis.call('TYPE', key)['ok']
    return kind == 'none' or kind == want
end

-- Returns every due row of `inflight` to the inbox, each on its own lease: a
-- redelivered message is marked, and may arrive after newer mail (a batch whose
-- lease is longer is not waited for).
--
-- Rows that ran out of redeliveries are dead-lettered, or, when the dead-letter
-- key cannot take them, counted as `stuck` and left where they are. Every other due row goes back to the front of
-- the inbox, marked one redelivery higher.
--
-- Returns {requeued, dead-lettered, stuck, evicted dead letters}. The caller has
-- checked that `inbox` and `inflight` are lists.
local function reap(inbox, inflight, dead, now, max_redeliveries, dead_cap)
    local dead_ok = readable(dead, 'list')
    local back, buried, gone, stuck = {}, {}, {}, 0
    for _, row in ipairs(redis.call('LRANGE', inflight, 0, -1)) do
        local id, who, deadline, entry = parse(row)
        if id and deadline > now then
            -- still leased: left alone
        elseif not id then
            if dead_ok then
                buried[#buried + 1] = row
                gone[#gone + 1] = row
            else
                stuck = stuck + 1
            end
        else
            local core, n = split_count(entry)
            if n >= max_redeliveries then
                if dead_ok then
                    buried[#buried + 1] = core
                    gone[#gone + 1] = row
                else
                    stuck = stuck + 1
                end
            else
                back[#back + 1] = core .. string.rep(' ', n + 1)
                gone[#gone + 1] = row
            end
        end
    end
    if #gone == 0 then return { 0, 0, stuck, 0 } end
    -- Destinations first, sources last.
    for _, entry in ipairs(buried) do redis.call('RPUSH', dead, entry) end
    for i = #back, 1, -1 do redis.call('LPUSH', inbox, back[i]) end
    for _, row in ipairs(gone) do redis.call('LREM', inflight, 1, row) end
    local evicted = 0
    if #buried > 0 then
        local over = redis.call('LLEN', dead) - dead_cap
        if over > 0 then
            evicted = over
            redis.call('LTRIM', dead, -dead_cap, -1)
        end
    end
    return { #back, #buried, stuck, evicted }
end
