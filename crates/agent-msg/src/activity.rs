//! Hook activity events (FR-068): which hook payload becomes which event, the
//! final-message classifier, and the branch and skill-version reads.
//!
//! Nothing here keeps tool input, tool output, a prompt or message text; the
//! only message-derived value is the [`LastClass`].

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use relay_core::{ActivityEvent, ActivityKind, BlockedReason, Harness, LastClass};

use crate::hook::{Event, HookInput};

/// The role spec the session loaded, set by whatever starts the session.
pub(crate) const ROLE_ENV: &str = "AGENT_MSG_ROLE";
/// Space- or comma-separated `name=version` pairs of the skills the session
/// loaded; a name with no version is recorded as `unknown`.
pub(crate) const SKILLS_ENV: &str = "AGENT_MSG_SKILLS";

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
            let (role, skills) = session_versions(
                std::env::var(ROLE_ENV).ok().as_deref(),
                std::env::var(SKILLS_ENV).ok().as_deref(),
            );
            e.role = Some(role);
            e.skill_versions = skills;
            e
        }
        Event::SessionEnd => make(ActivityKind::SessionEnd),
    };
    e.cwd = input.cwd.clone().unwrap_or_default();
    Some(e)
}

/// The role and `name=version` pairs from the session's environment values.
/// An unset role is `unknown`; a skill without a version gets `unknown`.
pub(crate) fn session_versions(role: Option<&str>, skills: Option<&str>) -> (String, Vec<String>) {
    let role = role
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .unwrap_or("unknown");
    let pairs = skills
        .unwrap_or_default()
        .split([' ', ',', '\n'])
        .filter(|s| !s.is_empty())
        .map(|s| match s.split_once('=') {
            Some((name, version)) if !version.is_empty() => format!("{name}={version}"),
            Some((name, _)) => format!("{name}=unknown"),
            None => format!("{s}=unknown"),
        })
        .collect();
    (role.to_owned(), pairs)
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
    use super::*;

    fn input(json: &str) -> HookInput {
        serde_json::from_str(json).unwrap()
    }

    /// Trace: FR-068-AC-4
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

    /// Trace: FR-068-AC-4
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

    /// Trace: FR-068-AC-3
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

    /// Trace: FR-068-AC-5
    #[test]
    fn session_versions_pair_every_skill_and_default_a_missing_version() {
        let (role, skills) = session_versions(Some("coder=1.2"), Some("a=1 b, c=  d="));
        assert_eq!(role, "coder=1.2");
        assert_eq!(skills, ["a=1", "b=unknown", "c=unknown", "d=unknown"]);
        let (role, skills) = session_versions(None, None);
        assert_eq!(role, "unknown");
        assert!(skills.is_empty());
    }

    /// Trace: FR-068-AC-1
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

    /// Trace: FR-068-AC-10
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

    /// Trace: FR-068-AC-10
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
