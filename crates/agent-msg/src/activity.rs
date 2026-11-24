//! Hook activity events (FR-068): which hook payload becomes which event, the
//! final-message classifier, and the branch and skill-version reads.
//!
//! Nothing here keeps tool input, tool output, a prompt or message text; the
//! only message-derived value is the [`LastClass`].

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use relay_core::{ActivityEvent, ActivityKind, BlockedReason, Harness, LastClass};

use crate::hook::{Event, HookInput};

/// The role spec the session loaded, set by whatever launches the session.
pub(crate) const ROLE_ENV: &str = "AGENT_MSG_ROLE";
/// The plugin whose installed version is the session's skill version.
const SKILL_PLUGIN: &str = "dev-team";

/// The longest a branch read may take before the branch is reported empty.
pub(crate) const BRANCH_READ_LIMIT: Duration = Duration::from_millis(100);

/// Phrases that make a final message a question when its last paragraph
/// holds one (lowercase).
const QUESTION_PHRASES: [&str; 7] = [
    "should i",
    "shall i",
    "want me to",
    "would you like",
    "do you want",
    "please confirm",
    "which option",
];

/// The event this hook payload produces, with `cwd` and `branch` still to
/// fill in at write time; `None` when it produces none (an event that is not
/// an activity kind, a notification of another type, no session id).
pub(crate) fn from_input(
    event: Event,
    input: &HookInput,
    harness: Harness,
    ts: i64,
) -> Option<ActivityEvent> {
    let session = input.session_id.as_ref()?.as_str();
    let make = |kind| ActivityEvent::new(kind, harness, session, ts);
    let mut e = match event {
        Event::PreToolUse => {
            let mut e = make(ActivityKind::PreTool);
            e.tool = Some(input.tool_name.clone().unwrap_or_default());
            e
        }
        Event::PostToolUse => {
            let mut e = make(ActivityKind::PostTool);
            e.tool = Some(input.tool_name.clone().unwrap_or_default());
            e
        }
        Event::UserPromptSubmit => make(ActivityKind::Prompt),
        Event::Notification => match input.notification_type.as_deref() {
            Some("permission_prompt") => {
                let mut e = make(ActivityKind::Blocked);
                e.reason = Some(BlockedReason::Permission);
                e
            }
            Some("idle_prompt") => make(ActivityKind::Idle),
            _ => return None,
        },
        Event::Stop => {
            let mut e = make(ActivityKind::Stop);
            e.last_class = Some(
                input
                    .last_assistant_message
                    .as_deref()
                    .map_or(LastClass::Unknown, classify),
            );
            e
        }
        Event::SessionStart => {
            let mut e = make(ActivityKind::SessionStart);
            e.role = Some(role_from(std::env::var(ROLE_ENV).ok().as_deref()));
            e.skill_versions = Some(plugin_versions(harness, &PluginHomes::from_env()));
            e
        }
        Event::SessionEnd => make(ActivityKind::SessionEnd),
    };
    e.cwd = input.cwd.clone().unwrap_or_default();
    Some(e)
}

/// The role spec: `AGENT_MSG_ROLE` as the launcher set it, `unknown` only when
/// it is unset or empty.
pub(crate) fn role_from(role: Option<&str>) -> String {
    role.map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}

/// Where the harnesses keep their installed plugins.
pub(crate) struct PluginHomes {
    /// The user's home directory.
    pub(crate) home: Option<PathBuf>,
    /// `CODEX_HOME`, when set.
    pub(crate) codex_home: Option<PathBuf>,
}

impl PluginHomes {
    fn from_env() -> Self {
        Self {
            home: std::env::var_os("HOME").map(PathBuf::from),
            codex_home: std::env::var_os("CODEX_HOME").map(PathBuf::from),
        }
    }
}

/// The skill version of the session: the installed `dev-team` plugin version,
/// from Claude's install manifest (`plugins/installed_plugins.json`, key
/// `dev-team@agent-ix`) or Codex's plugin cache
/// (`plugins/cache/agent-ix/dev-team/<version>`). A version that cannot be read
/// is `unknown`, never omitted.
pub(crate) fn plugin_versions(harness: Harness, homes: &PluginHomes) -> BTreeMap<String, String> {
    let version = match harness {
        Harness::Claude => homes.home.as_ref().and_then(|home| {
            claude_plugin_version(&home.join(".claude/plugins/installed_plugins.json"))
        }),
        Harness::Codex => homes
            .codex_home
            .clone()
            .or_else(|| homes.home.as_ref().map(|home| home.join(".codex")))
            .and_then(|dir| {
                codex_plugin_version(&dir.join("plugins/cache/agent-ix").join(SKILL_PLUGIN))
            }),
    };
    BTreeMap::from([(
        SKILL_PLUGIN.to_owned(),
        version.unwrap_or_else(|| "unknown".to_owned()),
    )])
}

fn claude_plugin_version(manifest: &Path) -> Option<String> {
    let mut text = String::new();
    fs::File::open(manifest)
        .ok()?
        .take(1 << 20)
        .read_to_string(&mut text)
        .ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let installs = value.get("plugins")?.get("dev-team@agent-ix")?.as_array()?;
    installs
        .iter()
        .rev()
        .find_map(|install| install.get("version")?.as_str().map(str::to_owned))
}

/// The highest version directory under the plugin's Codex cache folder.
fn codex_plugin_version(plugin_dir: &Path) -> Option<String> {
    let key = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    fs::read_dir(plugin_dir)
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with(|c: char| c.is_ascii_digit()))
        .max_by_key(|name| key(name))
}

/// Whether the final assistant message asks the user something. Reads the
/// message only and keeps none of it.
pub(crate) fn classify(message: &str) -> LastClass {
    let trimmed = message.trim_end_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '\u{201d}' | '\u{2019}' | ')' | ']' | '}' | '*' | '_' | '`' | '~'
            )
    });
    if trimmed.ends_with('?') {
        return LastClass::Question;
    }
    let last_paragraph = message
        .trim()
        .rsplit("\n\n")
        .find(|p| !p.trim().is_empty())
        .unwrap_or_default()
        .to_lowercase();
    if QUESTION_PHRASES.iter().any(|p| last_paragraph.contains(p)) {
        LastClass::Question
    } else {
        LastClass::Done
    }
}

/// The checked-out branch of `cwd`, or empty when `cwd` is not in a git work
/// tree, `HEAD` is detached, or reading takes longer than `limit`.
pub(crate) fn branch_within(cwd: &str, limit: Duration) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    let cwd = PathBuf::from(cwd);
    let (tx, rx) = mpsc::channel();
    // A hung read (a FIFO standing in for HEAD, a stalled mount) strands this
    // thread; the hook process exits right after, so it is never joined.
    thread::spawn(move || {
        let _ = tx.send(read_branch(&cwd));
    });
    rx.recv_timeout(limit).unwrap_or_default()
}

fn read_branch(cwd: &Path) -> String {
    let Some(git_dir) = cwd.ancestors().find_map(git_dir_of) else {
        return String::new();
    };
    let mut head = String::new();
    let Ok(file) = fs::File::open(git_dir.join("HEAD")) else {
        return String::new();
    };
    if file.take(1024).read_to_string(&mut head).is_err() {
        return String::new();
    }
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .unwrap_or_default()
        .to_owned()
}

/// The git directory of the work tree rooted at `dir`: `.git` itself, or the
/// directory a `.git` file of a linked work tree names.
fn git_dir_of(dir: &Path) -> Option<PathBuf> {
    let dot_git = dir.join(".git");
    let meta = fs::metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git);
    }
    let mut text = String::new();
    fs::File::open(&dot_git)
        .ok()?
        .take(4096)
        .read_to_string(&mut text)
        .ok()?;
    let target = Path::new(text.trim().strip_prefix("gitdir: ")?);
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        dir.join(target)
    })
}

#[cfg(test)]
mod tests {
    use ix_trace_rs::trace;

    use super::*;

    fn input(json: &str) -> HookInput {
        serde_json::from_str(json).unwrap()
    }

    #[trace("FR-068-AC-4")]
    #[test]
    fn the_final_message_is_classified_by_its_ending_and_last_paragraph() {
        let cases = [
            ("Shall we go on?", LastClass::Question),
            ("He said \"ready?\"", LastClass::Question),
            ("Is it **done?**", LastClass::Question),
            ("(really?)  \n", LastClass::Question),
            (
                "First part.\n\nI can ship it. Want me to open the PR.",
                LastClass::Question,
            ),
            ("Want me to?\n\nNo, finished. Merged.", LastClass::Done),
            ("All done. The tests pass.", LastClass::Done),
        ];
        for (text, want) in cases {
            assert_eq!(classify(text), want, "{text:?}");
        }
    }

    #[trace("FR-068-AC-4")]
    #[test]
    fn a_stop_event_carries_the_class_and_never_the_message() {
        let secret = "SECRET-MARKER-9f3a";
        for (message, want) in [
            (Some(format!("{secret} anything?")), "question"),
            (Some(format!("{secret} finished.")), "done"),
            (None, "unknown"),
        ] {
            let mut raw = serde_json::json!({
                "session_id": "s-1", "hook_event_name": "Stop", "cwd": "/w",
            });
            if let Some(m) = message {
                raw["last_assistant_message"] = m.into();
            }
            let i = input(&raw.to_string());
            let e = from_input(Event::Stop, &i, Harness::Claude, 7).unwrap();
            let json = e.encode();
            assert!(!json.contains(secret), "{json}");
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(v["last_class"], want);
        }
    }

    #[trace("FR-068-AC-3")]
    #[test]
    fn notifications_become_blocked_idle_or_nothing() {
        let notify = |t: &str| {
            let i = input(&format!(
                r#"{{"session_id":"s-1","hook_event_name":"Notification","notification_type":"{t}"}}"#
            ));
            from_input(Event::Notification, &i, Harness::Claude, 7)
        };
        let blocked: serde_json::Value =
            serde_json::from_str(&notify("permission_prompt").unwrap().encode()).unwrap();
        assert_eq!(blocked["kind"], "blocked");
        assert_eq!(blocked["reason"], "permission");
        let idle: serde_json::Value =
            serde_json::from_str(&notify("idle_prompt").unwrap().encode()).unwrap();
        assert_eq!(idle["kind"], "idle");
        assert!(notify("auth_success").is_none());
        assert!(
            from_input(
                Event::Notification,
                &input(r#"{"session_id":"s-1","hook_event_name":"Notification"}"#),
                Harness::Claude,
                7
            )
            .is_none()
        );
    }

    #[trace("FR-068-AC-5")]
    #[test]
    fn the_role_and_skill_version_come_from_the_launcher_and_the_installed_plugin() {
        assert_eq!(role_from(Some(" coder ")), "coder");
        assert_eq!(role_from(Some("")), "unknown");
        assert_eq!(role_from(None), "unknown");

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let homes = PluginHomes {
            home: Some(home.clone()),
            codex_home: None,
        };
        // Nothing installed: the version is present and `unknown`.
        assert_eq!(
            plugin_versions(Harness::Claude, &homes),
            BTreeMap::from([("dev-team".to_owned(), "unknown".to_owned())])
        );
        fs::create_dir_all(home.join(".claude/plugins")).unwrap();
        fs::write(
            home.join(".claude/plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{
                "other@x":[{"version":"9.9.9"}],
                "dev-team@agent-ix":[{"version":"0.3.0"},{"version":"0.4.1"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            plugin_versions(Harness::Claude, &homes)["dev-team"],
            "0.4.1"
        );

        // Codex keeps one directory per installed version.
        for v in ["0.4.1", "0.10.0", "0.9.2"] {
            fs::create_dir_all(home.join(format!(".codex/plugins/cache/agent-ix/dev-team/{v}")))
                .unwrap();
        }
        assert_eq!(
            plugin_versions(Harness::Codex, &homes)["dev-team"],
            "0.10.0"
        );
    }

    /// A `SessionStart` always carries both fields.
    #[trace("FR-068-AC-5")]
    #[test]
    fn a_session_start_event_always_carries_a_role_and_skill_versions() {
        let i = input(r#"{"session_id":"s-1","hook_event_name":"SessionStart","cwd":"/w"}"#);
        let e = from_input(Event::SessionStart, &i, Harness::Claude, 7).unwrap();
        let v: serde_json::Value = serde_json::from_str(&e.encode()).unwrap();
        assert!(v["role"].is_string());
        assert!(v["skill_versions"]["dev-team"].is_string(), "{v}");
    }

    #[trace("FR-068-AC-1")]
    #[test]
    fn a_tool_event_keeps_the_tool_name_and_no_input_or_output() {
        let i = input(
            r#"{"session_id":"s-1","hook_event_name":"PostToolUse","cwd":"/w","tool_name":"Bash",
                "tool_input":{"command":"echo SECRET-MARKER"},"tool_response":"SECRET-MARKER"}"#,
        );
        let json = from_input(Event::PostToolUse, &i, Harness::Codex, 7)
            .unwrap()
            .encode();
        assert!(!json.contains("SECRET-MARKER"), "{json}");
        assert!(json.contains(r#""tool":"Bash""#));
    }

    #[trace("FR-068-AC-10")]
    #[test]
    fn the_branch_is_read_from_head_and_empty_outside_a_work_tree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("sub/dir")).unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/swm-202-x\n").unwrap();
        let sub = repo.join("sub/dir");
        assert_eq!(
            branch_within(sub.to_str().unwrap(), Duration::from_secs(2)),
            "swm-202-x"
        );
        // A linked work tree names its git directory in a `.git` file.
        let linked = dir.path().join("linked");
        fs::create_dir_all(&linked).unwrap();
        let gitdir = dir.path().join("gd");
        fs::create_dir_all(&gitdir).unwrap();
        fs::write(gitdir.join("HEAD"), "ref: refs/heads/other\n").unwrap();
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .unwrap();
        assert_eq!(
            branch_within(linked.to_str().unwrap(), Duration::from_secs(2)),
            "other"
        );
        // Detached HEAD, no repository, and no cwd are all empty.
        fs::write(repo.join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(
            branch_within(repo.to_str().unwrap(), Duration::from_secs(2)),
            ""
        );
        let bare = dir.path().join("plain");
        fs::create_dir_all(&bare).unwrap();
        assert_eq!(
            branch_within(bare.to_str().unwrap(), Duration::from_secs(2)),
            ""
        );
        assert_eq!(branch_within("", Duration::from_secs(2)), "");
    }

    #[trace("FR-068-AC-10")]
    #[test]
    fn a_branch_read_that_hangs_gives_an_empty_branch_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        // A FIFO with no writer blocks every open of HEAD for reading.
        let made = std::process::Command::new("mkfifo")
            .arg(repo.join(".git/HEAD"))
            .status()
            .unwrap();
        assert!(made.success());
        let started = std::time::Instant::now();
        assert_eq!(
            branch_within(repo.to_str().unwrap(), Duration::from_millis(100)),
            ""
        );
        assert!(started.elapsed() < Duration::from_millis(900));
    }
}
