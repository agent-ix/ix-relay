#!/usr/bin/env bash
# Runs the workspace tests against a throwaway redis:7 container on a random
# localhost port, then removes the container.
set -euo pipefail

id=$(docker run -d --rm -p 127.0.0.1::6379 redis:7)
trap 'docker stop "$id" >/dev/null' EXIT

port=$(docker port "$id" 6379/tcp | head -n1 | sed 's/.*://')
for _ in $(seq 50); do
    docker exec "$id" redis-cli ping >/dev/null 2>&1 && break
    sleep 0.1
done

IX_RELAY_TEST_URL="redis://127.0.0.1:${port}" cargo test --workspace "$@"
