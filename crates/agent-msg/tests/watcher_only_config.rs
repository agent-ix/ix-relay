//! A typo in a watcher-only setting of `config.toml` must never stop `agent-msg`
//! or a hook (FR-045): the shared file type keeps `[watcher] otlp_endpoint`,
//! `otlp_headers` and `otlp_interval_secs` as raw values, and only
//! `relay-watcher` validates them.

#![expect(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure signal"
)]

mod support;

use support::{base_cmd, run};

const SESSION_START: &str = r#"{"hook_event_name":"SessionStart","session_id":"s1","cwd":"/work"}"#;

/// Watcher-only values `relay-watcher` refuses.
const BAD_FILES: [&str; 6] = [
    "[watcher]\notlp_endpoint = 5\n",
    "[watcher]\notlp_endpoint = \"https://collector:4318\"\n",
    "[watcher]\notlp_headers = \"novalue\"\notlp_interval_secs = 1\n",
    "[watcher]\notlp_interval_secs = \"abc\"\n",
    "[watcher]\notlp_interval_secs = 9223372036854775807\n",
    "[watcher]\notlp_interval_secs = 3601\n",
];

fn config_in(dir: &std::path::Path, text: &str) -> std::path::PathBuf {
    let path = dir.join("config.toml");
    std::fs::write(&path, text).expect("write the config");
    path
}

fn command(
    args: &[&str],
    name: Option<&str>,
    config: &std::path::Path,
    redis: Option<(&str, &str)>,
) -> std::process::Command {
    let mut cmd = base_cmd(args, name);
    cmd.env("IX_RELAY_CONFIG", config);
    if let Some((url, prefix)) = redis {
        cmd.env("IX_RELAY_URL", url);
        cmd.env("IX_RELAY_PREFIX", prefix);
    }
    cmd
}

/// Tracing: TC-045-02
/// ACs: FR-045-AC-2
///
/// With nothing listening for Redis, the hook still exits 0 and `who` reports
/// Redis, never the config file: the bad watcher-only value was not read as an
/// error. Mutation: typing the shared fields (`String`, `u64`) makes the
/// file malformed for every command and fails this.
#[test]
fn a_bad_watcher_only_key_never_makes_the_file_malformed_for_agent_msg() {
    for text in BAD_FILES {
        let dir = tempfile::tempdir().unwrap();
        let config = config_in(dir.path(), text);
        let hook = run(
            command(&["hook", "claude"], Some("qsl-lead"), &config, None),
            SESSION_START,
        );
        assert_eq!(hook.status, 0, "{text:?}: {}", hook.stderr);
        assert!(!hook.stderr.contains("otlp"), "{text:?}: {}", hook.stderr);
        let who = run(command(&["who"], Some("qsl-lead"), &config, None), "");
        assert!(
            !who.stderr.contains("config.toml"),
            "{text:?}: {}",
            who.stderr
        );
        assert!(!who.stderr.contains("otlp"), "{text:?}: {}", who.stderr);
    }
}

/// Tracing: TC-045-02
/// ACs: FR-045-AC-2
///
/// Against a real Redis: with a bad watcher-only value in the file, registering,
/// sending, delivering through the hook and `who` all work. Redis lane.
#[test]
fn delivery_and_who_work_with_a_bad_watcher_only_key() {
    let Ok(url) = std::env::var("IX_RELAY_TEST_URL") else {
        eprintln!("skipping: IX_RELAY_TEST_URL unset (run `make test-redis`)");
        return;
    };
    for (n, text) in BAD_FILES.iter().enumerate() {
        let prefix = format!("test:{}:watcher-only-{n}:", std::process::id());
        let dir = tempfile::tempdir().unwrap();
        let config = config_in(dir.path(), text);
        let redis = Some((url.as_str(), prefix.as_str()));
        let first = run(
            command(&["hook", "claude"], Some("receiver"), &config, redis),
            SESSION_START,
        );
        assert_eq!(first.status, 0, "{text:?}: {}", first.stderr);
        let sent = run(
            command(
                &["send", "receiver", "hi", "there"],
                Some("sender"),
                &config,
                redis,
            ),
            "",
        );
        assert_eq!(sent.status, 0, "{text:?}: {}", sent.stderr);
        let delivered = run(
            command(&["hook", "claude"], Some("receiver"), &config, redis),
            SESSION_START,
        );
        assert_eq!(delivered.status, 0, "{text:?}: {}", delivered.stderr);
        assert!(
            delivered.stdout.contains("hi there"),
            "{text:?}: {}",
            delivered.stdout
        );
        let who = run(command(&["who"], Some("receiver"), &config, redis), "");
        assert_eq!(who.status, 0, "{text:?}: {}", who.stderr);
    }
}
