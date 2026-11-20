#!/usr/bin/env bash
# Exercise the native deployment config with the installed Redis, without root/network ports.
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
scratch=$(mktemp -d)
redis_pid=
cleanup() {
  if [[ -n $redis_pid ]]; then kill "$redis_pid" 2>/dev/null || true; wait "$redis_pid" 2>/dev/null || true; fi
  rm -rf -- "$scratch"
}
trap cleanup EXIT
hash=$(printf test-password | sha256sum | cut -d' ' -f1)
printf 'user default off\nuser admin on #%s ~* &* +@all\n' "$hash" > "$scratch/users.acl"
sed -e 's/^bind .*/bind 127.0.0.1/' -e 's/^port .*/port 0/' \
  -e "s|/var/lib/ix-relay|$scratch|g" -e "s|/run/ix-relay/redis.sock|$scratch/redis.sock|" \
  "$root/deploy/redis-native.conf" > "$scratch/redis.conf"
cli() { REDISCLI_AUTH=test-password redis-cli --no-auth-warning --user admin -s "$scratch/redis.sock" "$@"; }
start() {
  redis-server "$scratch/redis.conf" > "$scratch/log" 2>&1 & redis_pid=$!
  for _ in {1..100}; do
    if [[ -S $scratch/redis.sock ]] && [[ $(cli PING 2>/dev/null) == PONG ]]; then return; fi
    sleep 0.05
  done
  cat "$scratch/log" >&2; return 1
}
start
[[ $(redis-cli -s "$scratch/redis.sock" PING) == *NOAUTH* ]]
[[ $(cli ACL SETUSER peer on '>peer-password' '~ixr:v3:*' resetchannels +@all -@admin -@dangerous) == OK ]]
[[ $(cli ACL SAVE) == OK ]]
[[ $(cli SET ixr:v3:test persistent) == OK ]]
# Normal SIGTERM exercises AOF shutdown and restart without a crash-loss window.
kill "$redis_pid"; wait "$redis_pid"; redis_pid=
start
peer() { REDISCLI_AUTH=peer-password redis-cli --no-auth-warning --user peer -s "$scratch/redis.sock" "$@"; }
[[ $(peer GET ixr:v3:test) == persistent ]]
[[ $(peer GET outside) == *NOPERM* ]]
[[ $(peer CONFIG GET '*') == *NOPERM* ]]
[[ $(peer EVAL "return redis.call('GET',KEYS[1])" 1 ixr:v3:test) == persistent ]]
[[ $(cli CONFIG GET maxmemory-policy | tail -n1) == noeviction ]]
echo 'PASS: native config, authentication, ACLs, Lua, and persistence across restart'
