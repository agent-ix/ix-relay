//! Is the `herdr` on this machine one ix-relay can work with?
//!
//! ix-relay reads herdr's CLI output with typed decoders (the watcher's
//! `agent get`, `find`'s `agent list`, `workspace list` and `tab list`) and herdr
//! owns that format. When herdr changes it, a decoder fails and a lead is
//! skipped with one log line. This check asks the same two questions up front
//! (FR-036): is the version at least [`MIN_HERDR_VERSION`], and do the responses
//! decode into the shapes ix-relay depends on? `relay-watcher` runs it at
//! start-up and again after a decode failure; `agent-msg doctor` runs it on
//! demand. It only reads.
//!
//! The shape structs here list every member ix-relay's decoders *require*, and
//! nothing else: a member herdr adds is not a mismatch, and a member herdr's own
//! schema makes optional (`agent_session`, which a pane reports only once the
//! harness integration has told herdr its session) is not required here either.
//! Typing into a pane is guarded per lead by the session comparison in the
//! watcher (`session_matches` refuses when herdr reports no id), so an absent
//! session costs that one lead a delivery, never the whole machine.

use std::{
    fmt,
    str::FromStr,
    time::{Duration, Instant},
};

use herdr_cli::{ProcessError, ProcessRunner};
use serde::Deserialize;

/// The oldest herdr version ix-relay supports, declared here and nowhere else:
/// the version its decoders were written against. (`herdr --version` prints
/// `herdr <major>.<minor>.<patch>`.)
pub const MIN_HERDR_VERSION: HerdrVersion = HerdrVersion::release(0, 9, 1);

/// The most characters of herdr's own text a [`HerdrCheck`] keeps.
const MAX_DETAIL_CHARS: usize = 160;

/// A herdr release, `major.minor.patch` with an optional pre-release tag. A
/// pre-release is older than the release it precedes (semver), so `0.9.1-rc1` is
/// below [`MIN_HERDR_VERSION`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrVersion {
    /// Major.
    pub major: u32,
    /// Minor.
    pub minor: u32,
    /// Patch.
    pub patch: u32,
    /// What follows a `-`: `rc1` in `0.9.1-rc1`. `None` for a release.
    pub pre: Option<String>,
}

impl HerdrVersion {
    /// A release `major.minor.patch`.
    #[must_use]
    pub const fn release(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: None,
        }
    }
}

impl Ord for HerdrVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for HerdrVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for HerdrVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl FromStr for HerdrVersion {
    type Err = ();

    /// `[v]major.minor.patch[-pre][+build]`, each number a number; a build suffix
    /// after `+` is ignored.
    fn from_str(text: &str) -> Result<Self, ()> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split('+').next().unwrap_or_default();
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, Some(pre.to_owned())),
            Some(_) => return Err(()),
            None => (text, None),
        };
        let mut parts = core.split('.').map(|p| p.parse::<u32>().map_err(|_| ()));
        let mut next = || parts.next().ok_or(())?;
        let version = Self {
            major: next()?,
            minor: next()?,
            patch: next()?,
            pre,
        };
        parts.next().is_none().then_some(version).ok_or(())
    }
}

/// The version in `herdr --version`'s output: the first word that parses.
fn version_in(output: &str) -> Option<HerdrVersion> {
    output.split_whitespace().find_map(|w| w.parse().ok())
}

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HerdrCheck {
    /// The check was not run (a test double, or a caller with no herdr to ask).
    NotChecked,
    /// herdr is new enough and answers in the shape ix-relay expects.
    Ok {
        /// The version it reported.
        version: HerdrVersion,
        /// Whether a response was there to check: `false` when herdr has no
        /// agents, so only the version was verified.
        shape_verified: bool,
    },
    /// The `herdr` binary could not be run.
    Missing {
        /// Why, bounded.
        reason: String,
    },
    /// herdr is older than [`MIN_HERDR_VERSION`].
    Unsupported {
        /// What it reported.
        found: HerdrVersion,
        /// What ix-relay needs.
        required: HerdrVersion,
    },
    /// `herdr --version` printed nothing this check can read as a version.
    VersionUnreadable {
        /// What it printed, bounded.
        output: String,
    },
    /// A response did not decode into the shape ix-relay needs.
    ShapeMismatch {
        /// The command whose output it was, e.g. `agent get`.
        command: &'static str,
        /// What failed to decode (names the missing field), bounded.
        detail: String,
    },
    /// herdr ran but a command failed or timed out: nothing is known about its
    /// shape.
    Failed {
        /// Why, bounded.
        reason: String,
    },
}

impl HerdrCheck {
    /// Whether ix-relay can rely on this herdr as far as the check can tell.
    /// [`HerdrCheck::NotChecked`] counts: nothing was found wrong.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::NotChecked | Self::Ok { .. })
    }

    /// Whether the check found something wrong: herdr missing, too old, or
    /// answering in a shape ix-relay cannot read. [`HerdrCheck::Failed`] is not a
    /// finding: a timeout, or a herdr server that is not up yet, says nothing
    /// about herdr, so it is "unknown" everywhere (the watcher's heartbeat and
    /// notification, `doctor` locally and remotely) and is re-checked.
    #[must_use]
    pub fn is_problem(&self) -> bool {
        !matches!(
            self,
            Self::NotChecked | Self::Ok { .. } | Self::Failed { .. }
        )
    }

    /// Whether the watcher must type into no pane: only when a member it cannot
    /// type without (the pane, the agent's status and change counter) does not
    /// decode, so nothing it reads can be trusted. A member herdr's schema makes
    /// optional is never this: the per-lead session guard already refuses to type
    /// when herdr reports no session id. An old version with a good shape, a
    /// missing binary or a timeout is advisory, not this.
    #[must_use]
    pub fn blocks_delivery(&self) -> bool {
        matches!(self, Self::ShapeMismatch { .. })
    }

    /// A stable code for what was found, for `doctor`'s JSON.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotChecked => "not_checked",
            Self::Ok { .. } => "ok",
            Self::Missing { .. } => "missing",
            Self::Unsupported { .. } => "unsupported_version",
            Self::VersionUnreadable { .. } => "version_unreadable",
            Self::ShapeMismatch { .. } => "shape_mismatch",
            Self::Failed { .. } => "failed",
        }
    }

    /// One printable line saying what is wrong, `None` when nothing is. Bounded
    /// and free of control characters: it goes into a notification, a heartbeat
    /// and a terminal.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        match self {
            Self::NotChecked | Self::Ok { .. } => None,
            Self::Missing { reason } => Some(format!("herdr cannot be run: {reason}")),
            Self::Unsupported { found, required } => Some(format!(
                "herdr {found} is older than the {required} ix-relay needs"
            )),
            Self::VersionUnreadable { output } => {
                Some(format!("herdr --version printed no version: {output:?}"))
            }
            Self::ShapeMismatch { command, detail } => Some(format!(
                "herdr {command} output has a shape ix-relay cannot read: {detail}"
            )),
            Self::Failed { reason } => Some(format!("herdr could not be checked: {reason}")),
        }
    }
}

/// `text` with control characters removed, cut to [`MAX_DETAIL_CHARS`].
fn bounded(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_DETAIL_CHARS)
        .collect()
}

/// The `herdr agent list` members ix-relay requires of every agent.
#[derive(Deserialize)]
struct ListShape {
    agents: Vec<ListedAgent>,
}

/// One listed agent's required members: what the watcher cannot type into a pane
/// without (which pane, and whether the agent is ready).
#[derive(Deserialize)]
#[expect(dead_code, reason = "the fields exist to be required by the decoder")]
struct ListedAgent {
    pane_id: String,
    tab_id: String,
    workspace_id: String,
    agent_status: String,
    state_change_seq: u64,
}

/// The `herdr agent get` members the watcher requires.
#[derive(Deserialize)]
#[expect(dead_code, reason = "the fields exist to be required by the decoder")]
struct GetShape {
    agent: GotAgent,
}

/// The agent inside a `get` reply.
#[derive(Deserialize)]
#[expect(dead_code, reason = "the fields exist to be required by the decoder")]
struct GotAgent {
    agent_status: String,
    state_change_seq: u64,
}

/// herdr's code for "no agent in that pane": not a shape problem.
const AGENT_NOT_FOUND: &str = "agent_not_found";

/// Runs the check with `runner`, each herdr call bounded by `timeout` and the
/// whole by three of them: [`check_within`].
#[must_use]
pub fn check(runner: &impl ProcessRunner, timeout: Duration) -> HerdrCheck {
    check_within(runner, timeout, timeout.saturating_mul(3))
}

/// Runs the check with `runner`, each herdr call bounded by `per_call` and the
/// whole by `budget`: a call that would start with the budget spent is not made,
/// and the check is [`HerdrCheck::Failed`].
///
/// Reads only: `herdr --version`, `herdr agent list` and, when there is a
/// listed pane, `herdr agent get <pane>`.
#[must_use]
pub fn check_within(
    runner: &impl ProcessRunner,
    per_call: Duration,
    budget: Duration,
) -> HerdrCheck {
    let started = Instant::now();
    let call = || per_call.min(budget.saturating_sub(started.elapsed()));
    if call().is_zero() {
        return spent();
    }
    let out = match runner.run("herdr", &["--version"], call()) {
        Ok(out) => out,
        Err(ProcessError::Spawn { reason, .. }) => {
            return HerdrCheck::Missing {
                reason: bounded(&reason),
            };
        }
        Err(e) => {
            return HerdrCheck::Failed {
                reason: bounded(&e.to_string()),
            };
        }
    };
    let printed = String::from_utf8_lossy(&out.stdout);
    if !out.success {
        return HerdrCheck::Failed {
            reason: bounded(&format!("--version exited {}", out.status_code)),
        };
    }
    let Some(version) = version_in(&printed) else {
        return HerdrCheck::VersionUnreadable {
            output: bounded(&printed),
        };
    };
    // The shape is checked whatever the version says: an old herdr with a good
    // shape stays advisory, and a bad shape is a bad shape at any version.
    match shape(runner, &call) {
        Err(problem) => problem,
        Ok(_) if version < MIN_HERDR_VERSION => HerdrCheck::Unsupported {
            found: version,
            required: MIN_HERDR_VERSION,
        },
        Ok(shape_verified) => HerdrCheck::Ok {
            version,
            shape_verified,
        },
    }
}

/// The check that ran out of budget before it could ask.
fn spent() -> HerdrCheck {
    HerdrCheck::Failed {
        reason: "the check's time budget was spent".to_owned(),
    }
}

/// Decodes what herdr answers into the shapes above: `Ok(true)` when a response
/// was checked, `Ok(false)` when there was none to check. `call` is the timeout
/// the next call may have.
fn shape(runner: &impl ProcessRunner, call: &dyn Fn() -> Duration) -> Result<bool, HerdrCheck> {
    if call().is_zero() {
        return Err(spent());
    }
    let listed: ListShape = decode(runner, "agent list", &["agent", "list"], call())?;
    let Some(first) = listed.agents.first() else {
        return Ok(false);
    };
    if call().is_zero() {
        return Err(spent());
    }
    match decode::<GetShape>(
        runner,
        "agent get",
        &["agent", "get", &first.pane_id],
        call(),
    ) {
        Ok(_) => Ok(true),
        // The agent left between the two calls: nothing wrong with the shape.
        Err(HerdrCheck::Failed { reason }) if reason.contains(AGENT_NOT_FOUND) => Ok(true),
        Err(problem) => Err(problem),
    }
}

/// One herdr call decoded as `T`, or the [`HerdrCheck`] it means.
fn decode<T: serde::de::DeserializeOwned>(
    runner: &impl ProcessRunner,
    command: &'static str,
    args: &[&str],
    timeout: Duration,
) -> Result<T, HerdrCheck> {
    herdr_cli::call::<T>(runner, "herdr", args, timeout).map_err(|error| match error {
        ProcessError::BadOutput { reason, .. } => HerdrCheck::ShapeMismatch {
            command,
            detail: bounded(&reason),
        },
        other => HerdrCheck::Failed {
            reason: bounded(&format!("{command}: {other}")),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use herdr_cli::Output;

    use super::*;

    /// One canned answer: the arguments it is for, and what the call returns.
    type Answer = (Vec<String>, Result<Output, ProcessError>);

    /// Answers each call by program arguments from a table.
    struct Stub(RefCell<Vec<Answer>>);

    impl Stub {
        fn new(answers: Vec<(&[&str], Result<Output, ProcessError>)>) -> Self {
            Self(RefCell::new(
                answers
                    .into_iter()
                    .map(|(args, out)| (args.iter().map(|s| (*s).to_owned()).collect(), out))
                    .collect(),
            ))
        }
    }

    impl ProcessRunner for Stub {
        fn run(&self, _: &str, args: &[&str], _: Duration) -> Result<Output, ProcessError> {
            let table = self.0.borrow();
            table
                .iter()
                .find(|(a, _)| a.iter().map(String::as_str).eq(args.iter().copied()))
                .map(|(_, out)| out.clone())
                .unwrap_or_else(|| {
                    Err(ProcessError::Spawn {
                        program: "herdr".into(),
                        reason: format!("no stub for {args:?}"),
                    })
                })
        }
    }

    fn ok(stdout: &str) -> Result<Output, ProcessError> {
        Ok(Output {
            status_code: 0,
            success: true,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }

    const LIST: &str = r#"{"id":"x","result":{"agents":[{"pane_id":"w1:p1","tab_id":"w1:t1",
        "workspace_id":"w1","agent_status":"idle","state_change_seq":4,"extra":1}]}}"#;
    const GET: &str = r#"{"result":{"agent":{"agent_status":"idle","state_change_seq":4}}}"#;

    fn supported() -> Stub {
        Stub::new(vec![
            (&["--version"], ok("herdr 0.9.1\n")),
            (&["agent", "list"], ok(LIST)),
            (&["agent", "get", "w1:p1"], ok(GET)),
        ])
    }

    const T: Duration = Duration::from_secs(1);

    #[test]
    fn versions_parse_and_order() {
        assert_eq!("0.9.1".parse(), Ok(MIN_HERDR_VERSION));
        assert_eq!("v0.9.1".parse(), Ok(MIN_HERDR_VERSION), "a leading v");
        assert_eq!(
            "0.9.1+build5".parse(),
            Ok(MIN_HERDR_VERSION),
            "build ignored"
        );
        let rc = "0.9.1-rc1".parse::<HerdrVersion>().unwrap();
        assert_eq!(rc.pre.as_deref(), Some("rc1"));
        assert!(rc < MIN_HERDR_VERSION, "a pre-release is below its release");
        assert!("0.9.2-rc1".parse::<HerdrVersion>().unwrap() > MIN_HERDR_VERSION);
        assert!("0.9.1-rc1".parse::<HerdrVersion>().unwrap() < "0.9.1-rc2".parse().unwrap());
        assert!("0.9.1-".parse::<HerdrVersion>().is_err());
        assert_eq!(rc.to_string(), "0.9.1-rc1");
        assert!("0.9".parse::<HerdrVersion>().is_err());
        assert!("0.9.1.2".parse::<HerdrVersion>().is_err());
        assert!("0.9.x".parse::<HerdrVersion>().is_err());
        assert!("0.9.0".parse::<HerdrVersion>().unwrap() < MIN_HERDR_VERSION);
        assert!("0.10.0".parse::<HerdrVersion>().unwrap() > MIN_HERDR_VERSION);
    }

    #[test]
    fn a_supported_herdr_passes_and_extra_members_are_no_mismatch() {
        assert_eq!(
            check(&supported(), T),
            HerdrCheck::Ok {
                version: MIN_HERDR_VERSION,
                shape_verified: true
            }
        );
    }

    #[test]
    fn a_herdr_with_no_agents_passes_on_its_version_alone() {
        let stub = Stub::new(vec![
            (&["--version"], ok("herdr 0.9.1")),
            (&["agent", "list"], ok(r#"{"result":{"agents":[]}}"#)),
        ]);
        assert_eq!(
            check(&stub, T),
            HerdrCheck::Ok {
                version: MIN_HERDR_VERSION,
                shape_verified: false
            }
        );
    }

    /// Tracing: TC-036-01
    /// ACs: FR-036-AC-1
    ///
    /// A herdr below the minimum with a good shape is named with both versions,
    /// and that is advisory: the watcher keeps delivering.
    #[test]
    fn tc_036_01_an_old_herdr_with_a_good_shape_is_named_with_both_versions_and_is_advisory() {
        let stub = Stub::new(vec![
            (&["--version"], ok("herdr 0.8.0")),
            (&["agent", "list"], ok(LIST)),
            (&["agent", "get", "w1:p1"], ok(GET)),
        ]);
        let found = check(&stub, T);
        assert!(found.is_problem() && !found.blocks_delivery());
        assert_eq!(found.code(), "unsupported_version");
        let text = found.problem().unwrap();
        assert!(text.contains("0.8.0") && text.contains("0.9.1"), "{text}");
    }

    /// A pre-release of the minimum is below it.
    #[test]
    fn a_prerelease_of_the_minimum_is_unsupported() {
        let stub = Stub::new(vec![
            (&["--version"], ok("herdr 0.9.1-rc1")),
            (&["agent", "list"], ok(LIST)),
            (&["agent", "get", "w1:p1"], ok(GET)),
        ]);
        assert_eq!(check(&stub, T).code(), "unsupported_version");
    }

    use relay_test_support::HERDR_LIST_ONE_CLAUDE_PANE_WITHOUT_A_SESSION as REAL_SHAPED_LIST;

    /// Tracing: TC-036-06
    /// ACs: FR-036-AC-6
    ///
    /// `agent_session` is optional in herdr's schema, and a claude pane whose
    /// harness has not reported a session omits it. That must not fail the check
    /// or block delivery for the machine: the per-lead session guard refuses that
    /// one pane. A check that required it (the mutant) fails here.
    #[test]
    fn tc_036_06_a_claude_pane_without_agent_session_beside_a_healthy_one_passes() {
        let get_p1 = r#"{"result":{"agent":{"agent":"claude","agent_status":"idle","state_change_seq":10}}}"#;
        let stub = Stub::new(vec![
            (&["--version"], ok("herdr 0.9.1")),
            (&["agent", "list"], ok(REAL_SHAPED_LIST)),
            (&["agent", "get", "w1:p1"], ok(get_p1)),
        ]);
        let found = check(&stub, T);
        assert!(found.is_ok(), "{found:?}");
        assert!(!found.blocks_delivery() && found.problem().is_none());
        assert_eq!(found.code(), "ok");
    }

    /// Only a shape ix-relay cannot read blocks delivery; every other outcome
    /// is advisory, and a timeout is not even a finding.
    #[test]
    fn only_a_shape_mismatch_blocks_delivery_and_a_timeout_is_unknown() {
        let timeout = Stub::new(vec![(
            &["--version"],
            Err(ProcessError::Timeout {
                program: "herdr".into(),
                secs: 1,
            }),
        )]);
        let failed = check(&timeout, T);
        assert!(!failed.is_problem() && !failed.blocks_delivery() && failed.problem().is_some());
        assert!(
            HerdrCheck::Missing {
                reason: String::new()
            }
            .is_problem()
        );
        assert!(
            !HerdrCheck::Missing {
                reason: String::new()
            }
            .blocks_delivery()
        );
        assert!(!HerdrCheck::NotChecked.is_problem());
    }

    /// A zero budget makes no call at all.
    #[test]
    fn a_check_with_no_budget_asks_nothing() {
        let stub = Stub::new(vec![]);
        let found = check_within(&stub, T, Duration::ZERO);
        assert_eq!(found.code(), "failed");
    }

    #[test]
    fn a_missing_binary_is_missing() {
        let stub = Stub::new(vec![]);
        assert_eq!(check(&stub, T).code(), "missing");
    }

    /// Tracing: TC-036-02
    /// ACs: FR-036-AC-2
    #[test]
    fn a_response_without_state_change_seq_names_the_field() {
        let stub = Stub::new(vec![
            (&["--version"], ok("herdr 0.9.1")),
            (&["agent", "list"], ok(LIST)),
            (
                &["agent", "get", "w1:p1"],
                ok(r#"{"result":{"agent":{"agent_status":"idle"}}}"#),
            ),
        ]);
        let found = check(&stub, T);
        assert_eq!(found.code(), "shape_mismatch");
        assert!(found.problem().unwrap().contains("state_change_seq"));
    }

    #[test]
    fn a_list_missing_a_required_member_is_a_mismatch() {
        let stub = Stub::new(vec![
            (&["--version"], ok("herdr 0.9.1")),
            (
                &["agent", "list"],
                ok(r#"{"result":{"agents":[{"pane_id":"w1:p1"}]}}"#),
            ),
        ]);
        assert_eq!(check(&stub, T).code(), "shape_mismatch");
    }

    #[test]
    fn an_agent_that_left_between_the_calls_is_not_a_mismatch() {
        let gone = Ok(Output {
            status_code: 1,
            success: false,
            stdout: Vec::new(),
            stderr: br#"{"error":{"code":"agent_not_found","message":"gone"}}"#.to_vec(),
        });
        let stub = Stub::new(vec![
            (&["--version"], ok("herdr 0.9.1")),
            (&["agent", "list"], ok(LIST)),
            (&["agent", "get", "w1:p1"], gone),
        ]);
        assert!(check(&stub, T).is_ok());
    }

    #[test]
    fn unreadable_version_output_and_a_timeout_are_reported_not_guessed() {
        let stub = Stub::new(vec![(&["--version"], ok("welcome to herdr"))]);
        assert_eq!(check(&stub, T).code(), "version_unreadable");
        let stub = Stub::new(vec![(
            &["--version"],
            Err(ProcessError::Timeout {
                program: "herdr".into(),
                secs: 1,
            }),
        )]);
        assert_eq!(check(&stub, T).code(), "failed");
    }

    #[test]
    fn a_problem_line_is_bounded_and_printable() {
        let noisy = format!("\u{1b}[31m{}", "x".repeat(1000));
        let stub = Stub::new(vec![(&["--version"], ok(&noisy))]);
        let text = check(&stub, T).problem().unwrap();
        assert!(!text.contains('\u{1b}'));
        assert!(text.chars().count() < 260);
    }
}
