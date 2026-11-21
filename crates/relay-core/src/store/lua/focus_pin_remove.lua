-- KEYS: record. ARGV: pin field, the value the caller read. Removes only that value.
if redis.call('HGET', KEYS[1], ARGV[1]) == ARGV[2] then
  redis.call('HDEL', KEYS[1], ARGV[1])
  return {'removed'}
end
return {'changed'}
