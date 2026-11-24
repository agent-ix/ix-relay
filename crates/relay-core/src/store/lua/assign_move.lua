-- A lane's list follows the lane to its new address, with a transition naming both; or, when it
-- cannot, a `move_failed` transition and a standing defect.
-- KEYS: 1 old list, 2 old meta, 3 lanes set, 4 events stream, 5 events start, 6 new list,
-- 7 new meta, 8 move defects hash.
-- ARGV: 1 old lane, 2 new lane, 3 writer, 4 order id (empty), 5 max events, 6 max age ms,
-- 7 events ttl seconds, 8 mode (`move`, or `fail` to record a failure the caller saw),
-- 9 the reason for `fail`.
-- Replies 1 moved; 0 the old lane holds no list; -8 the new lane holds one already (a
-- `move_failed` is recorded, nothing else changed); -7 wrong key type (nothing written);
-- 2 a `fail` was recorded.
local ms = now_ms('')
-- A recorded failure needs only the old lane, the history and the defects.
if not (readable(KEYS[1], 'list') and readable(KEYS[2], 'hash') and readable(KEYS[4], 'stream')
    and readable(KEYS[5], 'string') and readable(KEYS[8], 'hash')) then
    return -7
end
local failed = {meta = KEYS[2], list = KEYS[1], events = KEYS[4], start = KEYS[5], defects = KEYS[8]}
if ARGV[8] == 'fail' then
    assign_move_failed(failed, ARGV[1], ARGV[2], ARGV[3], ARGV[4], ARGV[9], ms, ARGV[5],
        tonumber(ARGV[6]), ARGV[7])
    return 2
end
if not (readable(KEYS[3], 'set') and readable(KEYS[6], 'list') and readable(KEYS[7], 'hash')) then
    return -7
end
if redis.call('EXISTS', KEYS[2]) == 0 then return 0 end
if redis.call('EXISTS', KEYS[6]) == 1 then
    assign_move_failed(failed, ARGV[1], ARGV[2], ARGV[3], ARGV[4], 'target_occupied', ms, ARGV[5],
        tonumber(ARGV[6]), ARGV[7])
    return -8
end
redis.call('DEL', KEYS[7])
if redis.call('EXISTS', KEYS[1]) == 1 then redis.call('RENAME', KEYS[1], KEYS[6]) end
redis.call('RENAME', KEYS[2], KEYS[7])
local version = redis.call('HINCRBY', KEYS[7], 'version', 1)
redis.call('HSET', KEYS[7], 'at_ms', string.format('%d', ms))
redis.call('SREM', KEYS[3], ARGV[1])
if redis.call('EXISTS', KEYS[6]) == 1 then redis.call('SADD', KEYS[3], ARGV[2]) end
redis.call('HDEL', KEYS[8], ARGV[1] .. '>' .. ARGV[2])
assign_log(KEYS[4], KEYS[5], assign_ids(redis.call('LRANGE', KEYS[6], 0, -1)), 'move', ARGV[2],
    ARGV[3], ARGV[4], ARGV[1], '', version, ms, ARGV[5], tonumber(ARGV[6]), ARGV[7])
return 1
