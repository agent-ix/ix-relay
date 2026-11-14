#!/usr/bin/env bash
# Regression: systemd must not treat a spawned Redis process as a ready socket.
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
docker run --rm -v "$root:/source:ro" --entrypoint bash redis:7 -ec '
mkdir -p /usr/local/libexec /etc/ix-relay /run/ix-relay /var/lib/ix-relay
# Match the host package binary path used by the production helper.
ln -s /usr/local/bin/redis-cli /usr/bin/redis-cli
cp /source/scripts/ix-relay-native-admin /source/scripts/ix-relay-wait-ready /usr/local/libexec/
printf test-password > /etc/ix-relay/admin.password
hash=$(sha256sum /etc/ix-relay/admin.password | cut -d" " -f1)
printf "user default off\nuser admin on #%s ~* &* +@all\n" "$hash" > /var/lib/ix-relay/users.acl
sed -e "s/^bind .*/bind 127.0.0.1/" -e "s/^port .*/port 0/" /source/deploy/redis-native.conf > /etc/ix-relay/redis.conf
(sleep 0.3; exec redis-server /etc/ix-relay/redis.conf) >/tmp/redis.log 2>&1 &
redis_pid=$!
trap "kill $redis_pid 2>/dev/null || true" EXIT
/usr/local/libexec/ix-relay-wait-ready
echo "PASS: authenticated readiness waits through delayed socket creation"
'
