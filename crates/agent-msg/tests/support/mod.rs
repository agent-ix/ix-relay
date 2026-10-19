//! Shared spawn/isolation helpers for `agent-msg` integration tests
//! (`tests/hook.rs`, `tests/inbox_json.rs`). Not a test binary itself: cargo
//! only auto-discovers direct children of `tests/` as integration test
//! crates, so this lives in a subdirectory (`tests/support/mod.rs`) the same
//! way a `tests/common/` helper module normally would.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a failed unwrap is the failure signal; included by several test binaries that each fire a different subset, so #[expect] would be unfulfilled in some"
)]

#[allow(
    dead_code,
    reason = "shared fixtures used by different integration binaries"
)]
pub mod location;

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

/// A temp directory that is removed when it goes out of scope, pass or fail,
/// so tests never leak directories into `/tmp`. Derefs to its path.
pub struct Scratch(tempfile::TempDir);

impl Scratch {
    /// A fresh directory whose name starts with `prefix`.
    // Not every test binary that compiles this module makes one.
    #[allow(
        dead_code,
        reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used; it cannot move to a support crate because base_cmd needs env!(CARGO_BIN_EXE_agent-msg), which cargo defines only for agent-msg's own test binaries"
    )]
    pub fn new(prefix: &str) -> Self {
        Self(
            tempfile::Builder::new()
                .prefix(prefix)
                .tempdir()
                .expect("create scratch dir"),
        )
    }

    /// The directory.
    #[allow(
        dead_code,
        reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used; it cannot move to a support crate because base_cmd needs env!(CARGO_BIN_EXE_agent-msg), which cargo defines only for agent-msg's own test binaries"
    )]
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.0.path()
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        self.0.path()
    }
}

/// Prefix of the per-run config directories [`nonexistent_config_path`] hands
/// out, which [`run`] removes once the process has exited.
const AUTO_CONFIG_DIR_PREFIX: &str = "ix-relay-agent-msg-test-";

/// What running the binary produced.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each reading a different subset of the fields (tests/hook_stdin.rs spawns its own child and reads none), so #[expect] is unfulfilled where they are read"
)]
pub struct Run {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// A config path guaranteed not to exist, unique per call, inside a directory
/// unique per call. The default `IX_RELAY_CONFIG` for every helper below, so a
/// developer's real `~/.ix/relay/config.toml` — or a leftover from an earlier
/// test run — can never be read by a test that didn't ask for it. The
/// directory matters too: the hook failure spool lives beside the config file
/// (`<dir>/spool/hook.jsonl`), so each run gets its own and never touches the
/// real home or another test's.
pub fn nonexistent_config_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!(
            "ix-relay-agent-msg-test-{}-{n}",
            std::process::id()
        ))
        .join("config.toml")
}

/// The failure spool that belongs to `config_path` (see
/// [`nonexistent_config_path`]).
// Only the spool tests read it.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used; it cannot move to a support crate because base_cmd needs env!(CARGO_BIN_EXE_agent-msg), which cargo defines only for agent-msg's own test binaries"
)]
pub fn spool_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .expect("config path has a parent")
        .join("spool")
        .join("hook.jsonl")
}

/// The `agent-msg` invocation every helper below builds on: sets
/// `AGENT_MSG_NAME`, and isolates every `IX_RELAY_*` var this binary reads —
/// clearing `IX_RELAY_URL`, `IX_RELAY_PREFIX`, `IX_RELAY_MACHINE`,
/// `IX_RELAY_HUMAN` and `IX_RELAY_WATCHER_INTERVAL_SECS` (so a lead session's
/// own exported env can't leak into a test that spawns this binary as a
/// child process — a `Command` inherits the parent's env by default) and
/// pointing `IX_RELAY_CONFIG` at a path that doesn't exist, unless the
/// caller overrides it afterward.
pub fn base_cmd(args: &[&str], name: Option<&str>) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_agent-msg"));
    cmd.args(args);
    match name {
        Some(name) => cmd.env("AGENT_MSG_NAME", name),
        None => cmd.env_remove("AGENT_MSG_NAME"),
    };
    // A port nothing listens on, not the default `127.0.0.1:6379`: on a
    // machine that has the bus tunnel up, a test that forgot to name its own
    // Redis would otherwise read and write the real bus. Tests that want Redis
    // set `IX_RELAY_URL` after this.
    cmd.env("IX_RELAY_URL", "redis://127.0.0.1:1");
    cmd.env_remove("IX_RELAY_PREFIX");
    cmd.env_remove("IX_RELAY_MACHINE");
    cmd.env_remove("IX_RELAY_HUMAN");
    cmd.env_remove("IX_RELAY_WATCHER_INTERVAL_SECS");
    cmd.env("IX_RELAY_CONFIG", nonexistent_config_path());
    cmd.env_remove("HERDR_PANE_ID");
    cmd.env_remove("CODEX_THREAD_ID");
    cmd.env_remove("HERDR_ENV");
    if matches!(name, Some("hub:EA:code" | "hub:EA:sender" | "hub:qsl:code")) {
        let config = cmd
            .get_envs()
            .find(|(key, _)| *key == "IX_RELAY_CONFIG")
            .and_then(|(_, v)| v)
            .unwrap();
        let directory = Path::new(config).parent().unwrap().to_owned();
        std::fs::create_dir_all(&directory).unwrap();
        write_transport_herdr(&directory);
        cmd.env("HERDR_ENV", "1")
            .env("IX_RELAY_MACHINE", "hub")
            .env(
                "CODEX_THREAD_ID",
                if matches!(name, Some("hub:EA:code" | "hub:qsl:code")) {
                    "s1"
                } else {
                    "sender-session"
                },
            )
            .env(
                "PATH",
                format!("{}:{}", directory.display(), std::env::var("PATH").unwrap()),
            );
    }
    cmd
}

/// Spawns `cmd`, feeding `stdin`, and collects its exit code, stdout and stderr.
pub fn run(cmd: Command, stdin: &str) -> Run {
    // The config directory `nonexistent_config_path` made for this run: the
    // hook may have created a spool in it, so remove it afterwards. A path a
    // test chose itself is the test's to clean up.
    let auto_dir = cmd
        .get_envs()
        .find(|(name, _)| *name == "IX_RELAY_CONFIG")
        .and_then(|(_, value)| value)
        .and_then(|value| Path::new(value).parent())
        .filter(|dir| {
            dir.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(AUTO_CONFIG_DIR_PREFIX))
        })
        .map(Path::to_path_buf);
    let out = run_inner(cmd, stdin);
    if let Some(dir) = auto_dir {
        std::fs::remove_dir_all(dir).ok();
    }
    out
}

fn run_inner(mut cmd: Command, stdin: &str) -> Run {
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn agent-msg");
    // A silent hook (no name) exits without reading stdin, so the write can
    // race its exit and hit a closed pipe. That is expected, not a failure.
    let write = child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes());
    if let Err(e) = write {
        assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "write stdin: {e}");
    }
    let out = child.wait_with_output().expect("wait for agent-msg");
    Run {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).expect("utf8 stdout"),
        stderr: String::from_utf8(out.stderr).expect("utf8 stderr"),
    }
}

/// Runs `agent-msg` with `args`, feeding `stdin` and setting `AGENT_MSG_NAME`,
/// `IX_RELAY_URL` and `IX_RELAY_PREFIX` (each only when `Some`), isolated per
/// [`base_cmd`]. `agent_msg_with_pane` additionally sets `HERDR_PANE_ID`.
// Not every test binary that compiles this module calls it (`tests/pane_machine.rs`
// builds its own command), so it is dead code in those alone.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used; it cannot move to a support crate because base_cmd needs env!(CARGO_BIN_EXE_agent-msg), which cargo defines only for agent-msg's own test binaries"
)]
pub fn agent_msg(
    args: &[&str],
    name: Option<&str>,
    url: Option<&str>,
    prefix: Option<&str>,
    stdin: &str,
) -> Run {
    agent_msg_with_pane(args, name, url, prefix, None, stdin)
}

/// Same as [`agent_msg`], additionally setting `HERDR_PANE_ID` when `pane` is `Some`.
// Same reason as `agent_msg`.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used; it cannot move to a support crate because base_cmd needs env!(CARGO_BIN_EXE_agent-msg), which cargo defines only for agent-msg's own test binaries"
)]
pub fn agent_msg_with_pane(
    args: &[&str],
    name: Option<&str>,
    url: Option<&str>,
    prefix: Option<&str>,
    pane: Option<&str>,
    stdin: &str,
) -> Run {
    let mut cmd = base_cmd(args, name);
    if let Some(url) = url {
        cmd.env("IX_RELAY_URL", url);
    }
    if let Some(prefix) = prefix {
        cmd.env("IX_RELAY_PREFIX", prefix);
    }
    match pane {
        Some(pane) => cmd.env("HERDR_PANE_ID", pane),
        None => cmd.env_remove("HERDR_PANE_ID"),
    };
    run(cmd, stdin)
}

/// Runs `agent-msg` with `IX_RELAY_CONFIG` pointing at `config_path` — the
/// one place a test supplies its own config path instead of [`base_cmd`]'s
/// nonexistent default — and every other `IX_RELAY_*` var isolated per
/// [`base_cmd`], so the config file is the only thing that can supply them.
// Only `tests/hook.rs`'s config-file cases call this; `tests/inbox_json.rs`
// compiles this module too but doesn't need it, so it would otherwise be
// flagged dead in that binary alone.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used; it cannot move to a support crate because base_cmd needs env!(CARGO_BIN_EXE_agent-msg), which cargo defines only for agent-msg's own test binaries"
)]
pub fn agent_msg_with_config(
    args: &[&str],
    name: Option<&str>,
    config_path: &Path,
    stdin: &str,
) -> Run {
    let mut cmd = base_cmd(args, name);
    cmd.env("IX_RELAY_CONFIG", config_path);
    // The config file is the only thing that may supply the URL here.
    cmd.env_remove("IX_RELAY_URL");
    run(cmd, stdin)
}

/// Writes a fake `herdr` shell script into a fresh temp dir and returns that
/// dir, for a test to prepend to `PATH` (AGE-2018): a real herdr binary is
/// never needed to test naming derivation, only a stand-in that answers
/// `workspace list` / `tab list --workspace <id>` / `pane list --workspace
/// <id>` / `agent get <pane>` with the given JSON `result` bodies, appends
/// every call it received (one line, the full argv) to `call_log`, and —
/// when `fail` is set — refuses every call with an error envelope instead,
/// so a test can exercise "herdr fails, the hook stays silent". `agent get`
/// reports `agent_not_found` (a clean "no herdr agent name override" —
/// tier 3 of AGE-2018's precedence chain falls through to the herdr
/// location) unless a case in `agent_get_cases` matches its target pane id.
// Only `tests/naming.rs` calls this; `tests/hook.rs` and
// `tests/inbox_json.rs` compile this module too but don't need it.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used; it cannot move to a support crate because base_cmd needs env!(CARGO_BIN_EXE_agent-msg), which cargo defines only for agent-msg's own test binaries"
)]
pub fn fake_herdr_dir(
    workspaces_json: &str,
    tabs_json: &str,
    panes_json: &str,
    agent_get_cases: &str,
    call_log: &Path,
    fail: bool,
) -> Scratch {
    let scratch = Scratch::new("ix-relay-fake-herdr-");
    let dir = scratch.path().to_path_buf();
    let script = dir.join("herdr");
    let body = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
echo "$*" >> {call_log:?}
if [ "{fail_flag}" = "1" ]; then
  echo '{{"error":{{"code":"fake_herdr_failure","message":"fake herdr failure"}}}}' >&2
  exit 1
fi
case "$1 $2" in
  "workspace list") echo '{{"result":{{"workspaces":{workspaces_json}}}}}' ;;
  "tab list") echo '{{"result":{{"tabs":{tabs_json}}}}}' ;;
  "pane list") echo '{{"result":{{"panes":{panes_json}}}}}' ;;
  "agent get")
    case "$3" in
      {agent_get_cases}
      *) echo '{{"error":{{"code":"agent_not_found","message":"no agent"}}}}' >&2; exit 1 ;;
    esac
    ;;
  *) echo '{{"error":{{"code":"unsupported","message":"fake herdr: unsupported command"}}}}' >&2; exit 1 ;;
esac
"#,
        call_log = call_log.display(),
        fail_flag = if fail { "1" } else { "0" },
    );
    std::fs::write(&script, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    scratch
}

/// Runs `agent-msg` with `herdr_dir` prepended to `PATH` and
/// `HERDR_PANE_ID`/`HERDR_TAB_ID`/`HERDR_WORKSPACE_ID` set (AGE-2018),
/// deliberately with no `AGENT_MSG_NAME` — the whole point is to prove
/// derivation without one. Every other `IX_RELAY_*` var isolated per
/// [`base_cmd`].
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used; it cannot move to a support crate because base_cmd needs env!(CARGO_BIN_EXE_agent-msg), which cargo defines only for agent-msg's own test binaries"
)]
pub fn agent_msg_with_herdr(
    args: &[&str],
    url: Option<&str>,
    prefix: Option<&str>,
    herdr_dir: &Path,
    pane_id: &str,
    tab_id: &str,
    workspace_id: &str,
    stdin: &str,
) -> Run {
    let mut cmd = base_cmd(args, None);
    if let Some(url) = url {
        cmd.env("IX_RELAY_URL", url);
    }
    if let Some(prefix) = prefix {
        cmd.env("IX_RELAY_PREFIX", prefix);
    }
    cmd.env("HERDR_PANE_ID", pane_id);
    cmd.env("HERDR_TAB_ID", tab_id);
    cmd.env("HERDR_WORKSPACE_ID", workspace_id);
    let existing_path = std::env::var("PATH").unwrap_or_default();
    cmd.env("PATH", format!("{}:{existing_path}", herdr_dir.display()));
    run(cmd, stdin)
}

/// Most times a test runs a hook again after one that ran out of time.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used"
)]
pub const OUT_OF_TIME_ATTEMPTS: usize = 8;

/// Runs `attempt` (given its number, from 1, for a fresh bus prefix) until it
/// returns `true`, and panics if none does.
///
/// A hook has a time budget of 300 to 900 ms (NFR-001) that a test cannot widen,
/// so on a loaded machine a hook can run out of time before it reaches what the
/// test is about, and records that (`budget_exhausted` or `timeout`) instead
/// (AGE-2059). An attempt returns `false` for exactly such a run; the outcome under test is asserted only
/// by a run that had the time. What a hook records when it runs out of time is
/// pinned deterministically, on an injected clock, by `hook_run`'s own tests.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used"
)]
pub fn until_in_time(mut attempt: impl FnMut(usize) -> bool) {
    for n in 1..=OUT_OF_TIME_ATTEMPTS {
        if attempt(n) {
            return;
        }
        eprintln!("attempt {n}: the hook ran out of time, trying again");
    }
    panic!("{OUT_OF_TIME_ATTEMPTS} hooks in a row ran out of time: this machine is too loaded");
}

/// Whether `records` (the spool of one hook run) are the record a hook leaves
/// when it ran out of time: exactly one, with the code `budget_exhausted` or
/// `timeout`. A run that left an out-of-time record next to anything else is a
/// failure of the test, not a skipped run.
#[allow(
    dead_code,
    reason = "shared by every agent-msg test binary, each using a different subset, so #[expect] is unfulfilled where the item is used"
)]
pub fn ran_out_of_time(records: &[serde_json::Value]) -> bool {
    let out_of_time = |r: &serde_json::Value| {
        matches!(
            r["error_kind"].as_str(),
            Some("budget_exhausted" | "timeout")
        )
    };
    let hit = records.iter().any(out_of_time);
    if hit {
        assert!(
            records.len() == 1,
            "an out-of-time run leaves exactly one record: {records:?}"
        );
    }
    hit
}

/// Genuine two-lead topology for transport tests that operate through a TCP proxy.
#[allow(dead_code, reason = "shared helper")]
pub fn transport_topology(cmd: &mut Command) -> Scratch {
    let dir = Scratch::new("ix-relay-transport-herdr-");
    let agents = r#"[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","agent_status":"idle","agent_session":{"kind":"id","value":"s1"}},{"pane_id":"w1:p2","tab_id":"w1:t2","workspace_id":"w1","agent_status":"idle","agent_session":{"kind":"id","value":"sender-session"}}]"#;
    let body = format!(
        r#"#!/usr/bin/env bash
case "$1 $2" in
"agent list") printf '%s\n' '{{"result":{{"agents":{agents}}}}}' ;;
"workspace list") printf '%s\n' '{{"result":{{"workspaces":[{{"workspace_id":"w1","label":"EA"}}]}}}}' ;;
"tab list") printf '%s\n' '{{"result":{{"tabs":[{{"tab_id":"w1:t1","label":"code"}},{{"tab_id":"w1:t2","label":"sender"}}]}}}}' ;;
*) exit 1 ;;
esac
"#
    );
    std::fs::write(dir.join("herdr"), body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.join("herdr"), std::fs::Permissions::from_mode(0o755)).unwrap();
    cmd.env("HERDR_ENV", "1")
        .env("IX_RELAY_MACHINE", "hub")
        .env("CODEX_THREAD_ID", "sender-session")
        .env(
            "PATH",
            format!("{}:{}", dir.display(), std::env::var("PATH").unwrap()),
        );
    dir
}

#[allow(dead_code, reason = "shared fixture")]
fn write_transport_herdr(directory: &Path) {
    let script = r#"#!/usr/bin/env python3
import sys,json
args=sys.argv[1:]
if '--session' in args:
 i=args.index('--session'); del args[i:i+2]
if '--machine' in args:
 i=args.index('--machine'); del args[i:i+2]
key=args[0]
if key=='machine':
 print(json.dumps([{'label':s,'enabled':True,'target':'fixture-'+s,'session':'default'} for s in ['hub','node-a','terra']]))
else:
 data={
 'agent': {'agents':[
  {'pane_id':'w1:p1','tab_id':'w1:t1','workspace_id':'w1','agent_status':'idle','agent_session':{'kind':'id','value':'s1'}},
  {'pane_id':'w1:p2','tab_id':'w1:t2','workspace_id':'w1','agent_status':'idle','agent_session':{'kind':'id','value':'sender-session'}}]},
 'workspace': {'workspaces':[{'workspace_id':'w1','label':'EA'}]},
 'tab': {'tabs':[{'tab_id':'w1:t1','label':'code'},{'tab_id':'w1:t2','label':'sender'}]}
 }
 print(json.dumps({'result':data[key]}))
"#;
    std::fs::write(
        directory.join("ssh"),
        r#"#!/usr/bin/env python3
import os,sys,shlex,subprocess
args=shlex.split(sys.argv[-1]);i=args.index('herdr');args=args[i+1:]
raise SystemExit(subprocess.run([os.path.join(os.path.dirname(__file__),'herdr'),*args]).returncode)
"#,
    )
    .unwrap();
    std::fs::write(directory.join("herdr"), script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        directory.join("ssh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::set_permissions(
        directory.join("herdr"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
}
