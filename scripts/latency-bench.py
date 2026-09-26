"""Latency harness behind the AGE-2028 PR figures (not a test, not run by CI).

Measures hook, `send` and watcher delivery latency of two release builds
against a throwaway Redis, with a proxy adding 20 ms each way (about 42 ms RTT).

    docker run -d --rm --name ix-relay-bench -p 127.0.0.1:0:6379 redis:7
    docker port ix-relay-bench 6379/tcp            # the port to pass below
    cargo build --release -p agent-msg -p relay-watcher   # the "after" build
    # build the "before" build (e.g. `git archive origin/main` into a scratch dir)
    BASE_BIN=/path/to/base/target/release NEW_BIN=$PWD/target/release \\
        python3 scripts/latency-bench.py <redis-port> [hook] [send] [watch]
    docker stop ix-relay-bench

Needs python3, docker (for redis-cli) and a `redis:7` container named
`ix-relay-bench` (set REDIS_CONTAINER to use another name).
"""
import socket, threading, time, json, os, subprocess, statistics, sys, tempfile, stat

NEW = os.environ['NEW_BIN']
BASE = os.environ['BASE_BIN']
CONTAINER = os.environ.get('REDIS_CONTAINER', 'ix-relay-bench')
PORT = int(sys.argv[1])
REDIS = ('127.0.0.1', PORT)
ONE_WAY = 0.020

def rcli(*args, inp=None):
    return subprocess.run(['docker', 'exec', '-i', CONTAINER, 'redis-cli', *args], input=inp, capture_output=True, text=True).stdout

class LatProxy:
    def __init__(self, one_way):
        self.one_way = one_way
        self.sock = socket.socket(); self.sock.bind(('127.0.0.1', 0)); self.sock.listen(64)
        self.port = self.sock.getsockname()[1]
        threading.Thread(target=self.accept, daemon=True).start()
    def accept(self):
        while True:
            c, _ = self.sock.accept()
            s = socket.create_connection(REDIS)
            for a, b in ((c, s), (s, c)):
                threading.Thread(target=self.pump, args=(a, b), daemon=True).start()
    def pump(self, a, b):
        try:
            while True:
                d = a.recv(65536)
                if not d: break
                release = time.monotonic() + self.one_way
                wait = release - time.monotonic()
                if wait > 0: time.sleep(wait)
                b.sendall(d)
        except OSError: pass
        finally:
            for x in (a, b):
                try: x.shutdown(socket.SHUT_RDWR)
                except OSError: pass

def env_for(prefix, url, extra=None):
    e = {k: v for k, v in os.environ.items() if not k.startswith(('IX_RELAY', 'AGENT_MSG', 'HERDR'))}
    e.update({'IX_RELAY_URL': url, 'IX_RELAY_PREFIX': prefix, 'IX_RELAY_MACHINE': 'hub',
              'IX_RELAY_CONFIG': tempfile.mkdtemp(prefix='bench-cfg-') + '/config.toml'})
    if extra: e.update(extra)
    return e

def reg_json(name, pane, sid, place=True, ts=1):
    r = {'name': name, 'machine': 'hub', 'kind': 'claude', 'session_id': sid, 'cwd': '/work', 'herdr_pane': pane,
         'place': {'workspace': 'EA', 'tab': 'code', 'tabs_in_workspace': 2} if place else None, 'last_seen_ms': ts}
    return json.dumps(r)

def seed(prefix, extra_leads=0):
    rcli('--scan', '--pattern', prefix + '*')
    keys = rcli('--scan', '--pattern', prefix + '*').split()
    if keys: rcli('DEL', *keys)
    cmds = [f"HSET {prefix}agents receiver '{reg_json('receiver','w1:p1','s1')}'",
            f"SET {prefix}session:s1 receiver"]
    for i in range(extra_leads):
        cmds.append(f"HSET {prefix}agents lead-{i} '{reg_json(f'lead-{i}', f'w9:p{i}', f's-{i}')}'")
    rcli(inp='\n'.join(cmds) + '\n')

def direct_send(binary, prefix, text='hi'):
    e = env_for(prefix, f'redis://127.0.0.1:{PORT}', {'AGENT_MSG_NAME': 'sender'})
    subprocess.run([binary + '/agent-msg', 'send', 'receiver', text], env=e, capture_output=True)

def timed(cmd, env, stdin=''):
    t = time.perf_counter()
    r = subprocess.run(cmd, env=env, input=stdin, capture_output=True, text=True)
    return (time.perf_counter() - t) * 1000, r

def pct(xs, p):
    xs = sorted(xs); return xs[min(len(xs) - 1, int(round(p * (len(xs) - 1))))]

def hook_scenarios(proxy):
    out = {}
    for label, bin_ in (('before', BASE), ('after', NEW)):
        prefix = f'bench:{label}:hook:'
        for name, event, explicit, queued in (('UserPromptSubmit derived, 1 message', 'UserPromptSubmit', False, 1),
                                              ('UserPromptSubmit explicit, 1 message', 'UserPromptSubmit', True, 1),
                                              ('PostToolUse derived, empty', 'PostToolUse', False, 0),
                                              ('PostToolUse explicit, empty', 'PostToolUse', True, 0)):
            times, delivered = [], 0
            for i in range(20):
                seed(prefix)
                if queued: direct_send(bin_, prefix, 'hello')
                extra = {'HERDR_PANE_ID': 'w1:p1', 'HERDR_TAB_ID': 'w1:t1', 'HERDR_WORKSPACE_ID': 'w1'}
                if explicit: extra['AGENT_MSG_NAME'] = 'receiver'
                e = env_for(prefix, f'redis://127.0.0.1:{proxy.port}', extra)
                stdin = json.dumps({'hook_event_name': event, 'session_id': 's1', 'cwd': '/work'})
                ms, r = timed([bin_ + '/agent-msg', 'hook', 'claude'], e, stdin)
                times.append(ms)
                if 'hello' in r.stdout: delivered += 1
            out[(name, label)] = (statistics.median(times), pct(times, 0.95), delivered, 20 if queued else None)
    return out

def send_scenario(proxy):
    out = {}
    for label, bin_ in (('before', BASE), ('after', NEW)):
        prefix = f'bench:{label}:send:'
        seed(prefix, extra_leads=1000)
        times = []
        for i in range(15):
            e = env_for(prefix, f'redis://127.0.0.1:{proxy.port}', {'AGENT_MSG_NAME': 'sender'})
            ms, r = timed([bin_ + '/agent-msg', 'send', 'receiver', 'hi'], e)
            assert r.returncode == 0, r.stderr
            times.append(ms)
        out[label] = (statistics.median(times), pct(times, 0.95))
    return out

def fake_herdr():
    d = tempfile.mkdtemp(prefix='bench-herdr-')
    log = d + '/prompts.log'
    open(log, 'w').close()
    script = f'''#!/usr/bin/env bash
case "$1 $2" in
  "agent get") echo '{{"result":{{"agent":{{"agent_status":"idle","state_change_seq":1,"agent_session":{{"value":"s1"}}}},"type":"agent_info"}}}}' ;;
  "agent prompt") date +%s.%N >> {log} ;;
  *) echo '{{"error":{{"code":"unsupported","message":"fake"}}}}' >&2; exit 1 ;;
esac
'''
    p = d + '/herdr'; open(p, 'w').write(script); os.chmod(p, 0o755)
    return d, log

def watcher_scenario():
    out = {}
    for label, bin_ in (('before', BASE), ('after', NEW)):
        prefix = f'bench:{label}:watch:'
        seed(prefix)
        d, log = fake_herdr()
        e = env_for(prefix, f'redis://127.0.0.1:{PORT}')
        e['PATH'] = d + ':' + e['PATH']
        e['HOME'] = tempfile.mkdtemp(prefix='bench-home-')
        w = subprocess.Popen([bin_ + '/relay-watcher'], env=e, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(2.5)
        # idle load: commands processed over 10 s with nothing happening
        def total():
            for l in rcli('info', 'stats').splitlines():
                if l.startswith('total_commands_processed'): return int(l.split(':')[1])
        a = total(); time.sleep(10); b = total()
        idle_cmds_per_s = (b - a - 3) / 10  # minus our own INFO calls
        lat = []
        for i in range(15):
            time.sleep(0.37 + (i % 5) * 0.11)
            before_lines = len(open(log).read().split())
            t0 = time.time()
            direct_send(bin_, prefix, f'w{i}')
            deadline = t0 + 6
            while time.time() < deadline:
                ls = open(log).read().split()
                if len(ls) > before_lines:
                    lat.append((float(ls[before_lines]) - t0) * 1000); break
                time.sleep(0.005)
            else:
                lat.append(float('inf'))
        w.terminate(); w.wait()
        out[label] = (statistics.median(lat), pct(lat, 0.95), idle_cmds_per_s)
    return out

if __name__ == '__main__':
    proxy = LatProxy(ONE_WAY)
    which = sys.argv[2:] or ['hook', 'send', 'watch']
    if 'hook' in which:
        for (name, label), (med, p95, deliv, n) in sorted(hook_scenarios(proxy).items()):
            print(f'HOOK {name:40s} {label:6s} median {med:6.1f} ms  p95 {p95:6.1f} ms  delivered {deliv}/{n}')
    if 'send' in which:
        for label, (med, p95) in send_scenario(proxy).items():
            print(f'SEND by name, 1001 leads, 42ms link  {label:6s} median {med:6.1f} ms p95 {p95:6.1f} ms')
    if 'watch' in which:
        for label, (med, p95, idle) in watcher_scenario().items():
            print(f'WATCHER {label:6s} delivery median {med:7.1f} ms  p95 {p95:7.1f} ms   idle redis cmds/s {idle:.2f}')
