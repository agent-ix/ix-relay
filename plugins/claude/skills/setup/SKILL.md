---
name: setup
description: Use when setting up, adding, re-keying or fixing a machine's connection to the ix-relay bus: the WireGuard hub, a new Linux or macOS machine joining it, the relay-watcher and agent-msg install, or a machine whose `agent-msg doctor` says redis_unreachable. Covers Linux and macOS variants.
---

# ix-relay: setting up a machine on the bus

The bus is a Redis (relay-redis) on one **hub** machine, reached by every
machine, hub included, at `10.99.0.1:6380` over **WireGuard**. Each machine
has its own WireGuard key and its own Redis user. Background and design:
README.md "Transport".

| Name | Value |
|---|---|
| hub | the machine that runs relay-redis; peers reach it at its LAN address |
| WireGuard net | `10.99.0.0/24`, hub `10.99.0.1`, peers `10.99.0.2`… |
| WireGuard port | UDP `51820` (LAN only for now; off-LAN is AGE-2068) |
| bus address | `10.99.0.1:6380` (TCP; the only port a peer reaches over `wg0`) |
| hub admin tool | `scripts/relay-admin` in an ix-relay checkout on the hub |

## Rules

- **A private key never leaves its machine.** Make each key on the machine it
  belongs to. Only public keys travel (44 characters ending in `=`).
- **A bus URL is a secret**: `redis://<machine>:<password>@10.99.0.1:6380`.
  Never put one on the bus, in a ticket, in a PR or in chat. Hand it over with
  a file only the owner can read (e.g. `~/.ix/relay/peers/<machine>.url`,
  mode 600, on the hub) fetched over SSH.
- **sudo steps are the human's.** Give them one step at a time, with every
  value filled in, and wait for the result before the next. Don't chain a
  dozen steps into one message.
- Check before and after each step (`nc`, `agent-msg doctor`); measure,
  don't assume.

## 1. The hub (once)

On the hub, from an ix-relay checkout:

```bash
ix local up ix-relay --from-source                       # relay-redis (helm/), namespace relay
cp deploy/relay-redis-proxy.{socket,service} ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now relay-redis-proxy.socket   # 10.99.0.1:6380 -> relay-redis
sudo apt install wireguard-tools nftables
scripts/relay-admin hub-init                             # wg0 + its own nftables table
scripts/relay-admin add-user <hub>                       # the hub's own bus URL
```

`hub-init` writes `/etc/wireguard/wg0.conf` (the hub key never printed),
`/etc/wireguard/ixrelay.nft` (over `wg0` only TCP 6380; UDP 51820 only from the
LAN) and enables `wg-quick@wg0`. It needs no ufw and changes nothing else on
the host. If a step asks for sudo, give the human the command instead.

Check: `ip -brief addr show wg0` shows `10.99.0.1/24`;
`systemctl is-active wg-quick@wg0` is `active`.

## 2. A new machine joins WireGuard

### 2a. Make its key, on the machine

**Linux**

```bash
sudo apt install wireguard-tools        # dnf/pacman: the same package name
sudo sh -c 'umask 077; wg genkey > /etc/wireguard/relay.key; wg pubkey < /etc/wireguard/relay.key'
```

The last line prints the public key.

**macOS, WireGuard app (recommended: survives sleep and reboot)**

1. Install **WireGuard** from the Mac App Store and open it.
2. **Add Empty Tunnel…**, name it `relay`. The app makes the key pair and shows
   the **Public key**; copy it.
3. **Save** without activating.

**macOS, command line (alternative)**: `brew install wireguard-tools`, then the
Linux key command with `/opt/homebrew/etc/wireguard` in place of
`/etc/wireguard` (create it with `sudo mkdir -m 700 -p`). `sudo wg-quick up
relay` does not come back after a reboot on its own; prefer the app.

### 2b. Add the peer, on the hub

```bash
scripts/relay-admin add-peer <machine> '<public key>'
```

It creates the machine's Redis user and WireGuard peer, then prints:

- the machine's **bus URL** (secret; see Rules),
- a **ready-to-run Linux command** that writes `/etc/wireguard/relay.conf` from
  the key made in 2a,
- the **lines for the macOS app**,
- the check.

A mistyped key is refused before anything changes. "already a peer" means it
went in the first time; if that output is lost, see "Re-key or reissue".

### 2c. Install the tunnel, on the machine

**Linux**: run the command `add-peer` printed, then:

```bash
sudo systemctl enable --now wg-quick@relay
```

**macOS app**: open the `relay` tunnel, **Edit**, keep the generated
`PrivateKey`, add the printed lines, so it reads:

```
[Interface]
PrivateKey = (the generated one)
Address = 10.99.0.N/32

[Peer]
PublicKey = <hub public key>
Endpoint = <hub LAN IP>:51820
AllowedIPs = 10.99.0.1/32
PersistentKeepalive = 25
```

**Save**, **Activate**, and under **Edit** turn on **On-Demand** (Wi-Fi and
Ethernet).

`AllowedIPs = 10.99.0.1/32` routes only the bus through the tunnel, never
the machine's other traffic. The endpoint is the hub's LAN IP, as `add-peer`
prints it; set `RELAY_ENDPOINT` on the hub to print a DNS name instead.

### 2d. Check, on the machine

```bash
nc -z -w3 10.99.0.1 6380 && echo bus-reachable
```

`ping 10.99.0.1` never answers: the hub drops everything over `wg0` except
TCP 6380.

## 3. The machine joins the bus

### 3a. Install the binaries and the plugin

```bash
cargo install --git https://github.com/agent-ix/ix-relay agent-msg --locked
cargo install --git https://github.com/agent-ix/ix-relay relay-watcher --locked
```

(From a checkout: `cargo install --path crates/agent-msg --locked`, likewise
`crates/relay-watcher`.) Then install the Claude Code plugin
(`plugins/claude/README.md`) in each lead's sessions.

### 3b. Point it at the bus

Fetch the URL from the hub without it passing through anything else:

```bash
u=$(ssh <user>@<hub LAN IP> cat .ix/relay/peers/<machine>.url)
IX_RELAY_URL="$u" agent-msg doctor | head -n 1      # must say: answers
```

Then set it in `~/.ix/relay/config.toml` (other keys unchanged):

```bash
sed -i "s|^url = .*|url = \"$u\"|" ~/.ix/relay/config.toml      # Linux
sed -i '' "s|^url = .*|url = \"$u\"|" ~/.ix/relay/config.toml   # macOS (BSD sed)
```

A new machine with no `config.toml` yet: create it with `url = "…"` and
`machine = "<machine>"` (README.md "Configuration"); `chmod 600` it either way,
since it now holds a password.

### 3c. Run the watcher

**Linux (systemd user unit)**

```bash
cp deploy/relay-watcher.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now relay-watcher.service     # or: restart, if it was running
```

**macOS (launchd agent)**

```bash
sed "s/YOUR_USER/$USER/g" deploy/com.agent-ix.relay-watcher.plist \
  > ~/Library/LaunchAgents/com.agent-ix.relay-watcher.plist
launchctl load ~/Library/LaunchAgents/com.agent-ix.relay-watcher.plist
# already loaded: launchctl kickstart -k gui/$(id -u)/com.agent-ix.relay-watcher
```

### 3d. Check

```bash
agent-msg doctor
```

`redis: ... answers` and `watcher [<machine>]` ok mean it is on the bus. Lead
sessions pick the new URL up on their next hook; nothing needs restarting.

## Troubleshooting: what `doctor` says

`redis_unreachable` names the layer that failed:

| its cause says | check |
|---|---|
| refused this machine's user or password | the `url` in config.toml; reissue with `relay-admin add-user <machine>` |
| nothing listens there | hub: `systemctl --user enable --now relay-redis-proxy.socket` |
| accepted the connection and then dropped it | hub: `kubectl -n relay get pods`, `journalctl --user -u relay-redis-proxy.service` |
| nothing came back / WireGuard link | here: `sudo wg show` (Linux) or the app: a `latest handshake` older than 2 min means the link is down; hub: `systemctl status wg-quick@wg0` |
| no route | WireGuard is not up here: `sudo systemctl start wg-quick@relay`, or activate the app's tunnel |

`scripts/relay-admin status` on the hub shows `wg0`, the forwarder and
relay-redis at once.

## Re-key or reissue

On the hub:

| to | run |
|---|---|
| issue a new password (URL lost or leaked) | `relay-admin add-user <machine>` (the old URL stops working) |
| replace a machine's key (made elsewhere, lost, leaked) | make a new key on the machine (2a), then `relay-admin remove-peer <machine>` and `add-peer <machine> '<new key>'` |
| remove a machine | `relay-admin remove-peer <machine>`; on the machine, disable `wg-quick@relay` or delete the app's tunnel |

`admin` and `default` are relay-redis's own users and are refused as machine
names.
