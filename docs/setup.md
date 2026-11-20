# Setup

This guide puts machines on the ix-relay bus. The bus is a Redis, relay-redis, on one hub machine. Every machine, the hub included, reaches it at `10.99.0.1:6380` over WireGuard. Each machine has its own WireGuard key and its own Redis user.

Follow the sections in order for a new bus. To add a machine to an existing bus, start at [Add a machine](#add-a-machine).

## Reference values

| Item | Value |
|---|---|
| WireGuard network | `10.99.0.0/24`, hub at `10.99.0.1`, machines at `10.99.0.2` and up |
| WireGuard port | UDP 51820, accepted from the hub's LAN only |
| Bus address | `10.99.0.1:6380`, TCP. This is the only port a machine reaches over `wg0`. |
| Admin tool | `scripts/relay-admin` in an ix-relay checkout on the hub |
| Config file | `~/.ix/relay/config.toml` on every machine |

## Rules

- A private key stays on the machine that made it. Only public keys travel. A public key is 44 characters ending in `=`.
- A bus URL is a secret. It has the form `redis://<machine>:<password>@10.99.0.1:6380`. Keep it off the bus, out of tickets and out of chat. Hand it over as a file only its owner can read, for example `~/.ix/relay/peers/<machine>.url` with mode 600 on the hub, fetched over SSH.
- Steps that need `sudo` run one at a time, with every value filled in.
- Check after each step with `nc` or `agent-msg doctor`.

## Set up the hub

Run these on the hub, from an ix-relay checkout. The hub needs Docker, a local Kubernetes cluster managed with `ix local`, `wireguard-tools` and `nftables`.

1. Deploy relay-redis from the Helm chart in `helm/`:

   ```bash
   ix local up ix-relay --from-source
   ```

   The chart runs Redis 7 with append-only persistence on a volume in the `relay` namespace. The `default` user is off. Each machine gets an ACL user limited to `ixr:*` keys with `+@all -@admin -@dangerous`. The `admin` password is generated into the `relay-redis-admin` Secret and is used only inside the pod.

2. Install the forwarder that exposes relay-redis on the WireGuard address:

   ```bash
   cp deploy/relay-redis-proxy.{socket,service} ~/.config/systemd/user/
   systemctl --user daemon-reload
   systemctl --user enable --now relay-redis-proxy.socket
   ```

   The socket unit listens on `10.99.0.1:6380` and binds before `wg0` is up. The service unit relays each connection to the chart's NodePort (31379).

3. Create `wg0` and its firewall:

   ```bash
   sudo apt install wireguard-tools nftables
   scripts/relay-admin hub-init
   ```

   `hub-init` writes `/etc/wireguard/wg0.conf` without printing the hub key. It writes `/etc/wireguard/ixrelay.nft`, which allows only TCP 6380 over `wg0` and UDP 51820 from the LAN. It enables `wg-quick@wg0` and changes nothing else on the host. Pass `--lan <CIDR>` if it cannot detect the LAN.

4. Create the hub's own bus user:

   ```bash
   scripts/relay-admin add-user <hub>
   ```

   It prints the hub's bus URL.

5. Check. `ip -brief addr show wg0` shows `10.99.0.1/24`, and `systemctl is-active wg-quick@wg0` prints `active`. `scripts/relay-admin status` shows `wg0`, the forwarder and relay-redis together.

The hub accepts WireGuard from its LAN only. A machine outside the LAN needs a route to the hub's UDP 51820, and `RELAY_ENDPOINT` set on the hub so `add-peer` prints that name.

## Add a machine

### 1. Make the key on the machine

Linux:

```bash
sudo apt install wireguard-tools
sudo sh -c 'umask 077; wg genkey > /etc/wireguard/relay.key; wg pubkey < /etc/wireguard/relay.key'
```

The last line is the public key.

macOS, with the WireGuard app from the Mac App Store, which survives sleep and reboots:

1. Open the app and choose **Add Empty Tunnel...**.
2. Name the tunnel `relay` and copy its **Public key**.
3. Save without activating.

On macOS you can instead run `brew install wireguard-tools` and use the Linux key command with `/opt/homebrew/etc/wireguard`. A tunnel started with `sudo wg-quick up relay` does not return after a reboot.

### 2. Add the peer on the hub

```bash
scripts/relay-admin add-peer <machine> '<public key>'
```

This creates the machine's Redis user and its WireGuard peer. It prints the bus URL, a ready-to-run Linux command that writes `/etc/wireguard/relay.conf`, the lines to add to the macOS tunnel, and a check. A mistyped key is refused before anything changes. A key that is already a peer is refused as a duplicate.

`admin` and `default` are Redis's own users and cannot be machine names.

### 3. Install the tunnel on the machine

Linux: run the command that `add-peer` printed, then:

```bash
sudo systemctl enable --now wg-quick@relay
```

macOS: open the `relay` tunnel and choose **Edit**. Keep the generated `PrivateKey` and add the printed lines:

```
[Interface]
PrivateKey = (the generated key)
Address = 10.99.0.N/32

[Peer]
PublicKey = <hub public key>
Endpoint = <hub address>:51820
AllowedIPs = 10.99.0.1/32
PersistentKeepalive = 25
```

Save, choose **Activate**, and turn on **On-Demand** for Wi-Fi and Ethernet. `AllowedIPs = 10.99.0.1/32` routes only the bus through the tunnel.

### 4. Check the tunnel

```bash
nc -z -w3 10.99.0.1 6380 && echo bus-reachable
```

`ping 10.99.0.1` gets no answer, because the hub accepts only TCP 6380 over `wg0`.

## Join the bus

### 1. Install the binaries and the plugin

```bash
cargo install --git https://github.com/agent-ix/ix-relay agent-msg --locked
cargo install --git https://github.com/agent-ix/ix-relay relay-watcher --locked
```

From a checkout, use `cargo install --path crates/agent-msg --locked` and the same for `crates/relay-watcher`. Then install the plugin for each harness: [Claude Code](../plugins/claude/README.md), [Codex](../plugins/codex/README.md) or [opencode](../plugins/opencode/README.md).

### 2. Point the machine at the bus

Fetch the URL from the hub without passing it through any other channel, and test it before saving it:

```bash
u=$(ssh <user>@<hub address> cat .ix/relay/peers/<machine>.url)
IX_RELAY_URL="$u" agent-msg doctor | head -n 1     # the line says: answers
```

Save it in `~/.ix/relay/config.toml`:

```toml
url = "redis://<machine>:<password>@10.99.0.1:6380"
machine = "<machine>"
```

Run `chmod 600 ~/.ix/relay/config.toml`, because the file holds a password. [Configuration](configuration.md) lists every key.

### 3. Run the watcher

Linux, as a systemd user unit:

```bash
cp deploy/relay-watcher.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now relay-watcher.service
```

macOS, as a launchd agent:

```bash
sed "s/YOUR_USER/$USER/g" deploy/com.agent-ix.relay-watcher.plist \
  > ~/Library/LaunchAgents/com.agent-ix.relay-watcher.plist
launchctl load ~/Library/LaunchAgents/com.agent-ix.relay-watcher.plist
```

To restart a running watcher, use `systemctl --user restart relay-watcher` on Linux or `launchctl kickstart -k gui/$(id -u)/com.agent-ix.relay-watcher` on macOS.

The watcher retries Redis forever with exponential backoff from 1 s up to 30 s, and exits only on a command-line or configuration error. The units restart it after a crash. The systemd unit sets `StartLimitIntervalSec=0`, so a crash loop never leaves it in the `failed` state.

### 4. Check health

```bash
agent-msg doctor
```

The line `redis: ... answers` and an `ok` watcher line for the machine mean it is on the bus. Lead sessions pick up a new URL on their next hook, so no restart is needed. [How it works](how-it-works.md#health-reporting) lists the check levels.

## Troubleshoot

`redis_unreachable` names the layer that failed:

| Cause reported | Check |
|---|---|
| Redis refused this machine's user or password | The `url` in `config.toml`. Reissue it with `scripts/relay-admin add-user <machine>`. |
| Nothing listens at the address | On the hub: `systemctl --user enable --now relay-redis-proxy.socket`. |
| The connection was accepted and then dropped | On the hub: `kubectl -n relay get pods` and `journalctl --user -u relay-redis-proxy.service`. |
| Nothing came back, or the WireGuard link is down | On the machine: `sudo wg show` on Linux, or the app. A `latest handshake` older than 2 minutes means the link is down. On the hub: `systemctl status wg-quick@wg0`. |
| No route | WireGuard is not up on the machine. Run `sudo systemctl start wg-quick@relay`, or activate the app's tunnel. |

To inspect the watcher directly:

```bash
systemctl --user status relay-watcher                  # Linux
journalctl --user -u relay-watcher -e
launchctl list | grep relay                            # macOS
tail -n 50 ~/Library/Logs/relay-watcher.log
```

The watcher logs each outage once when it starts (`redis unreachable: ...` or `watcher cannot poll: ...`) and once when it ends (`redis is back after ...` or `watcher polling again after ...`). A per-lead condition such as blocked, stale registration, no agent or a Herdr error is also logged once at each end. After 60 seconds without a working poll it logs `no working poll for Ns; notifying` and shows one Herdr notification. Silence is normal.

`~/.ix/relay/state/watcher.json` records `pid`, `started_at_ms`, `last_ok_tick_ms` and `last_error`, so the state is readable while Redis is down. A `last_ok_tick_ms` more than about 20 seconds old with a running unit means the watcher cannot poll.

## Re-key, reissue or remove a machine

Run on the hub:

| Goal | Command |
|---|---|
| Issue a new password after the URL was lost or leaked | `scripts/relay-admin add-user <machine>`. The old URL stops working. |
| Replace a key that was lost, leaked or made elsewhere | Make a new key on the machine, then `scripts/relay-admin remove-peer <machine>` and `scripts/relay-admin add-peer <machine> '<new key>'`. |
| Remove a machine | `scripts/relay-admin remove-peer <machine>`. On the machine, disable `wg-quick@relay` or delete the app's tunnel. |

`scripts/relay-admin remove-user <machine>` removes the Redis user alone.


## Native systemd hub (Titan)

The native backend replaces Kubernetes and its TCP proxy with a dedicated host
Redis instance (tested with Redis 7 and Titan’s Redis 8.0.5). Install `redis-server`, `redis-tools`, `wireguard-tools`, and
`nftables`, then run `sudo scripts/install-native-redis`. This only stages the
service: it does not start Redis or change the existing bus. Run
`make test-native-redis` to verify the installed Redis against the native
configuration using an isolated temporary directory and Unix socket.

The service uses `/var/lib/ix-relay`, AOF every second, a 512 MiB Redis limit
with no eviction, and a 1 GiB systemd memory limit. Reserve at least 5 GiB disk.
Its admin password stays in `/etc/ix-relay/admin.password` (root only).
Use `IX_RELAY_BACKEND=systemd scripts/relay-admin ...` for native peer/user
administration. Kubernetes remains the default backend. Native `migrate` is
intentionally unavailable.

### Coordinated empty-bus move from Luna

Reserve Titan wired address `192.168.50.21` in DHCP. Inventory every peer and
save root-only copies of WireGuard configuration and each client's relay config
before changing anything. Do not publish private keys or client URLs.

Stop watchers on all machines, stop Luna's relay proxy socket **and service**,
scale its dedicated `ix-relay` deployment to zero, and disable its `wg0` hub.
Disable Titan's old `wg-quick@relay` peer before bringing up the new hub at the
same logical address. Leave all unrelated Luna services running.

On Titan run `IX_RELAY_BACKEND=systemd scripts/relay-admin hub-init --lan
192.168.50.0/24`, then `sudo systemctl enable --now ix-relay-redis`. The hub-init
firewall permits only destination `10.99.0.1:6380` through wg0 and blocks all
forwarding to/from wg0, independently of Docker's tables. Existing LAN access
is unchanged; no SSH credentials or access are added. The hub key is generated
on Titan. Do not reuse Luna's private key.

Enroll each remote machine using its existing public key and native `add-peer`;
set `RELAY_ENDPOINT=192.168.50.21` when generating peer instructions. Address
allocation follows enrollment order, so install the printed address on each
peer rather than assuming the previous assignment. Luna can reuse its existing
hub key locally as its new peer key, without transferring it. Use `add-user
titan` for Titan itself. Each machine gets a fresh Redis URL; install it in its
local config and remove stale URL environment overrides. Keep secrets out of
chat, tickets, logs, and shell history. Restart watchers and verify genuine live
Herdr sessions repopulate the directory without fabricated registrations.

Before retiring the old deployment, verify authenticated health on every peer,
message enqueue/delivery/acknowledgement in both directions, denial of other
tunnel ports and forwarding, denial of unauthenticated/admin Redis commands,
and Redis persistence across restart. Reboot Titan during the maintenance
window to check startup ordering. Observe 24 healthy hours before deleting the
old deployment. Monitor local logs, Redis latency, memory, disk/AOF failures,
watcher reconnects and hook budget failures independently of Luna.

Rollback means stopping Titan's bus and restoring the saved topology and client
configs on every machine together. Queued messages are disposable; never merge
the two buses. Do not run the legacy namespace migration for this move.


On Ubuntu hosts with the packaged WireGuard AppArmor profiles, keep staged
WireGuard configurations under `/etc/wireguard/` too. `wg-quick strip` cannot
read a configuration staged under `/etc/ix-relay/`, even as root. The hub's
PostUp hook pipes the firewall file through `cat` to `nft -f -`: wg-quick may
read `/etc/wireguard/`, while its confined nft child need not open that file.
Do not disable AppArmor to stage or activate the hub.

The native systemd unit waits for an authenticated Redis PING before start
completes (30-second readiness budget, 40-second systemd startup timeout).
A running process alone is not readiness: Redis may still be opening its
socket or loading AOF data. Re-run the staging installer after updating the
unit or its readiness helper; it does not start or restart services.
