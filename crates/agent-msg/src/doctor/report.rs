//! What `agent-msg doctor` reports: a list of [`Check`]s, each with a stable
//! [`Code`], and how it is printed (text for a person, JSON for a script).
//!
//! Every text a check carries can hold something another machine or a binary
//! wrote (a machine name, a herdr error, a spooled derived name). [`Check::new`]
//! is the one place they are made safe, so no check can print an escape code.

use std::io::{self, Write};

use relay_core::text::sanitise_line;
use serde::Serialize;

/// The most detail lines one check keeps.
const MAX_DETAILS: usize = 12;

/// The most characters of one detail line.
const MAX_LINE_CHARS: usize = 200;

/// The result of one check: its severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    /// Nothing wrong.
    Ok,
    /// Something needs attention now: `doctor` exits 1.
    Fail,
    /// Something is off but is expected to clear, is not urgent, or is not known
    /// to matter: reported, and `doctor` exits 0 unless run with `--strict`.
    Warn,
    /// It could not be told (no service manager to ask, herdr not answering).
    /// Reported, and never fails the exit.
    Unknown,
}

/// Which part of the relay a check is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Family {
    /// `config.toml`.
    Config,
    /// The Redis bus.
    Redis,
    /// A machine's watcher heartbeat.
    Watcher,
    /// `state/watcher.json`.
    WatcherFile,
    /// The local hook failure spool.
    HookSpool,
    /// Hook failures recorded on the bus.
    HookFailures,
    /// Panes moved and places renamed, as the watcher recorded them (FR-043).
    Moves,
    /// The `herdr` binary.
    Herdr,
    /// Registrations.
    Registration,
    /// In-flight batches.
    InFlight,
    /// Dead letters.
    DeadLetter,
    /// Inbox depth and age.
    Inbox,
    /// The systemd or launchd unit.
    Service,
    /// Delivery through Claude Code's inbox sockets (FR-044).
    Socket,
}

/// The outcome, as a stable code: `ok`, or what is wrong. A script keys on it; the
/// message and fix are prose for a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Code {
    /// The check passed.
    Ok,
    /// `config.toml` did not load.
    ConfigInvalid,
    /// Redis cannot be reached.
    RedisUnreachable,
    /// Redis answers, slowly.
    RedisSlow,
    /// A read of the bus failed after Redis answered.
    RedisReadFailed,
    /// A machine with recent leads has no watcher heartbeat.
    WatcherHeartbeatMissing,
    /// A heartbeat key is there and is not readable.
    WatcherHeartbeatUnreadable,
    /// A heartbeat's last tick is older than a live watcher's: the watcher is
    /// wedged, or the two machines' clocks disagree.
    WatcherHeartbeatStale,
    /// The heartbeat scan hit its bound and may have missed a machine.
    HeartbeatScanTruncated,
    /// `config.toml`'s watcher-only OTLP keys hold a value the watcher would refuse.
    WatcherOtlpConfigInvalid,
    /// A watcher reports herdr failing its check.
    WatcherHerdrProblem,
    /// A watcher cannot push metrics to its OTLP collector and is backing off.
    WatcherOtlpFailing,
    /// `watcher.json` is absent.
    WatcherFileMissing,
    /// There is no config location, so no `watcher.json` is looked for.
    WatcherFileUnlocated,
    /// `watcher.json` is not readable.
    WatcherFileUnreadable,
    /// `watcher.json`'s last good poll is old.
    WatcherFileStale,
    /// `watcher.json` names a process that is not running.
    WatcherPidGone,
    /// `watcher.json` and the heartbeat describe different processes.
    WatcherFileMismatch,
    /// Hook failures are waiting in the local spool.
    HookSpoolPending,
    /// There is no config location, so no spool is looked for.
    HookSpoolUnlocated,
    /// The local spool is not readable.
    HookSpoolUnreadable,
    /// Hook failures were recorded on the bus in the last 24 hours.
    HookFailuresRecorded,
    /// The recorded pane moves could not be read.
    MovesUnreadable,
    /// `herdr` cannot be run.
    HerdrMissing,
    /// herdr is older than ix-relay supports.
    HerdrUnsupportedVersion,
    /// herdr's output has a shape ix-relay cannot read: the watcher types into no
    /// pane.
    HerdrShapeMismatch,
    /// `herdr --version` printed no version.
    HerdrVersionUnreadable,
    /// herdr could not be checked (a failure or a timeout).
    HerdrUnchecked,
    /// A registration has not been seen for over a day.
    RegistrationStale,
    /// A registration with a pane has no place.
    RegistrationNoPlace,
    /// A registration's pane is not in herdr.
    RegistrationPaneGone,
    /// A registry record cannot be decoded.
    RegistrationUndecodable,
    /// herdr's panes could not be listed, so registrations were not checked
    /// against them.
    PanesUnchecked,
    /// Dead letters are waiting.
    DeadLettersPresent,
    /// In-flight leases are past their deadline and nothing has reaped them.
    InflightOverdue,
    /// The in-flight and dead-letter scan hit its bound.
    BacklogScanTruncated,
    /// A message has waited in an inbox longer than a delivery should take.
    InboxStuck,
    /// The watcher unit is not installed.
    ServiceNotInstalled,
    /// The watcher unit is installed and not running.
    ServiceInactive,
    /// The unit's state could not be read.
    ServiceUnknown,
    /// Claude Code holds a lead's relay mail behind an approval prompt.
    SocketHeld,
    /// Batches for a lead went to herdr because its socket could not be used.
    SocketFallback,
    /// The watcher dead-lettered a batch on its own account (denied, or held past the
    /// maximum hold).
    SocketDeadLettered,
    /// Socket delivery is on and the watcher's report is missing, unreadable or
    /// from a watcher that is gone.
    SocketStatusUnknown,
    /// Claude Code sent something on the reply socket, or a receipt status, that the typed
    /// wire formats do not know: a format may have changed.
    SocketWireDrift,
    /// A transcript line names a message id without a structured `origin.msg_id`: the
    /// transcript's shape may have changed; the batch is acknowledged as received but unconfirmed.
    SocketTranscriptShapeUnrecognised,
}

impl Code {
    /// The code as it is written in JSON and in the text report.
    pub(crate) fn name(self) -> String {
        wire_name(&self)
    }
}

/// The snake_case name serde gives an enum value.
pub(crate) fn wire_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Check {
    /// What it is about.
    pub check: Family,
    /// `ok`, or what is wrong.
    pub code: Code,
    /// Whether it passed.
    pub status: Status,
    /// The machine, lead or file it concerns, when it is one of several.
    pub subject: Option<String>,
    /// What was found.
    pub message: String,
    /// The likely cause, for a check that did not pass.
    pub cause: Option<String>,
    /// What to do about it.
    pub fix: Option<String>,
    /// Grouped counts and previews: hook failures by stage, a lost message's text.
    pub details: Vec<String>,
}

/// `text` as one safe, bounded line.
fn tidy(text: &str) -> String {
    sanitise_line(text).chars().take(MAX_LINE_CHARS).collect()
}

impl Check {
    /// A check that passed.
    pub(crate) fn ok(check: Family, subject: Option<&str>, message: impl AsRef<str>) -> Self {
        Self::new(check, Code::Ok, Status::Ok, subject, message.as_ref())
    }

    /// A failed check: `code` says what, `cause` why it is likely and `fix` what
    /// to do.
    pub(crate) fn fail(
        check: Family,
        code: Code,
        subject: Option<&str>,
        message: impl AsRef<str>,
        cause: &str,
        fix: &str,
    ) -> Self {
        Self::with_severity(
            Status::Fail,
            check,
            code,
            subject,
            message.as_ref(),
            cause,
            fix,
        )
    }

    /// A check that found something worth reporting and not worth failing on.
    pub(crate) fn warn(
        check: Family,
        code: Code,
        subject: Option<&str>,
        message: impl AsRef<str>,
        cause: &str,
        fix: &str,
    ) -> Self {
        Self::with_severity(
            Status::Warn,
            check,
            code,
            subject,
            message.as_ref(),
            cause,
            fix,
        )
    }

    /// `fail` or `warn` by `failing`: for a code whose severity depends on the
    /// facts (how recent, whether anything is waiting).
    pub(crate) fn severe(
        failing: bool,
        check: Family,
        code: Code,
        subject: Option<&str>,
        message: impl AsRef<str>,
        cause: &str,
        fix: &str,
    ) -> Self {
        let status = if failing { Status::Fail } else { Status::Warn };
        Self::with_severity(status, check, code, subject, message.as_ref(), cause, fix)
    }

    fn with_severity(
        status: Status,
        check: Family,
        code: Code,
        subject: Option<&str>,
        message: &str,
        cause: &str,
        fix: &str,
    ) -> Self {
        let mut found = Self::new(check, code, status, subject, message);
        found.cause = Some(tidy(cause));
        found.fix = Some(tidy(fix));
        found
    }

    /// A check that could not tell, with the reason and what would let it.
    pub(crate) fn unknown(
        check: Family,
        code: Code,
        subject: Option<&str>,
        message: impl AsRef<str>,
        fix: &str,
    ) -> Self {
        let mut found = Self::new(check, code, Status::Unknown, subject, message.as_ref());
        found.fix = Some(tidy(fix));
        found
    }

    fn new(
        check: Family,
        code: Code,
        status: Status,
        subject: Option<&str>,
        message: &str,
    ) -> Self {
        Self {
            check,
            code,
            status,
            subject: subject.map(tidy),
            message: tidy(message),
            cause: None,
            fix: None,
            details: Vec::new(),
        }
    }

    /// This check with detail lines (at most [`MAX_DETAILS`]).
    pub(crate) fn with_details(mut self, details: impl IntoIterator<Item = String>) -> Self {
        self.details = details
            .into_iter()
            .take(MAX_DETAILS)
            .map(|d| tidy(&d))
            .collect();
        self
    }
}

/// Everything `doctor` found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Report {
    /// Whether nothing failed: no `fail`, and with `--strict` no `warn` either.
    /// An `unknown` never fails it.
    pub ok: bool,
    /// Whether warnings count as failures (`--strict`).
    pub strict: bool,
    /// The checks, in the order they were made.
    pub checks: Vec<Check>,
}

impl Report {
    /// A report over `checks`.
    pub(crate) fn new(checks: Vec<Check>, strict: bool) -> Self {
        let bad = |c: &Check| c.status == Status::Fail || (strict && c.status == Status::Warn);
        Self {
            ok: !checks.iter().any(bad),
            strict,
            checks,
        }
    }

    /// Writes the report as text: a line per check, then its cause, fix and
    /// details indented under it.
    pub(crate) fn write_text(&self, out: &mut impl Write) -> io::Result<()> {
        for c in &self.checks {
            let tag = match c.status {
                Status::Ok => "ok",
                Status::Fail => "FAIL",
                Status::Warn => "warn",
                Status::Unknown => "unknown",
            };
            let subject = c
                .subject
                .as_deref()
                .map_or(String::new(), |s| format!(" [{s}]"));
            let what = if c.status == Status::Ok {
                wire_name(&c.check)
            } else {
                c.code.name()
            };
            writeln!(out, "{tag:>7}  {what}{subject}: {}", c.message)?;
            if let Some(cause) = &c.cause {
                writeln!(out, "         cause: {cause}")?;
            }
            if let Some(fix) = &c.fix {
                writeln!(out, "         fix: {fix}")?;
            }
            for d in &c.details {
                writeln!(out, "         - {d}")?;
            }
        }
        writeln!(
            out,
            "{}",
            if self.ok {
                "relay healthy"
            } else {
                "relay has problems"
            }
        )
    }

    /// Writes the report as one JSON line.
    pub(crate) fn write_json(&self, out: &mut impl Write) -> io::Result<()> {
        crate::json::write_line(out, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire name of every code. Exhaustive on purpose: a new [`Code`] must be
    /// named here, and a rename fails the pin below (scripts key on these).
    fn pinned(code: Code) -> &'static str {
        match code {
            Code::Ok => "ok",
            Code::ConfigInvalid => "config_invalid",
            Code::RedisUnreachable => "redis_unreachable",
            Code::RedisSlow => "redis_slow",
            Code::RedisReadFailed => "redis_read_failed",
            Code::WatcherHeartbeatMissing => "watcher_heartbeat_missing",
            Code::WatcherHeartbeatUnreadable => "watcher_heartbeat_unreadable",
            Code::WatcherHeartbeatStale => "watcher_heartbeat_stale",
            Code::HeartbeatScanTruncated => "heartbeat_scan_truncated",
            Code::WatcherOtlpConfigInvalid => "watcher_otlp_config_invalid",
            Code::WatcherHerdrProblem => "watcher_herdr_problem",
            Code::WatcherOtlpFailing => "watcher_otlp_failing",
            Code::WatcherFileMissing => "watcher_file_missing",
            Code::WatcherFileUnlocated => "watcher_file_unlocated",
            Code::WatcherFileUnreadable => "watcher_file_unreadable",
            Code::WatcherFileStale => "watcher_file_stale",
            Code::WatcherPidGone => "watcher_pid_gone",
            Code::WatcherFileMismatch => "watcher_file_mismatch",
            Code::HookSpoolPending => "hook_spool_pending",
            Code::HookSpoolUnlocated => "hook_spool_unlocated",
            Code::HookSpoolUnreadable => "hook_spool_unreadable",
            Code::HookFailuresRecorded => "hook_failures_recorded",
            Code::MovesUnreadable => "moves_unreadable",
            Code::HerdrMissing => "herdr_missing",
            Code::HerdrUnsupportedVersion => "herdr_unsupported_version",
            Code::HerdrShapeMismatch => "herdr_shape_mismatch",
            Code::HerdrVersionUnreadable => "herdr_version_unreadable",
            Code::HerdrUnchecked => "herdr_unchecked",
            Code::RegistrationStale => "registration_stale",
            Code::RegistrationNoPlace => "registration_no_place",
            Code::RegistrationPaneGone => "registration_pane_gone",
            Code::RegistrationUndecodable => "registration_undecodable",
            Code::PanesUnchecked => "panes_unchecked",
            Code::DeadLettersPresent => "dead_letters_present",
            Code::InflightOverdue => "inflight_overdue",
            Code::BacklogScanTruncated => "backlog_scan_truncated",
            Code::InboxStuck => "inbox_stuck",
            Code::ServiceNotInstalled => "service_not_installed",
            Code::ServiceInactive => "service_inactive",
            Code::ServiceUnknown => "service_unknown",
            Code::SocketHeld => "socket_held",
            Code::SocketFallback => "socket_fallback",
            Code::SocketDeadLettered => "socket_dead_lettered",
            Code::SocketStatusUnknown => "socket_status_unknown",
            Code::SocketWireDrift => "socket_wire_drift",
            Code::SocketTranscriptShapeUnrecognised => "socket_transcript_shape_unrecognised",
        }
    }

    /// Tracing: TC-029-06
    /// ACs: FR-029-AC-6
    #[test]
    fn tc_029_06_every_code_serialises_to_its_pinned_name() {
        for code in [
            Code::Ok,
            Code::ConfigInvalid,
            Code::RedisUnreachable,
            Code::RedisSlow,
            Code::RedisReadFailed,
            Code::WatcherHeartbeatMissing,
            Code::WatcherHeartbeatUnreadable,
            Code::WatcherHeartbeatStale,
            Code::HeartbeatScanTruncated,
            Code::WatcherOtlpConfigInvalid,
            Code::WatcherHerdrProblem,
            Code::WatcherOtlpFailing,
            Code::WatcherFileMissing,
            Code::WatcherFileUnlocated,
            Code::WatcherFileUnreadable,
            Code::WatcherFileStale,
            Code::WatcherPidGone,
            Code::WatcherFileMismatch,
            Code::HookSpoolPending,
            Code::HookSpoolUnlocated,
            Code::HookSpoolUnreadable,
            Code::HookFailuresRecorded,
            Code::MovesUnreadable,
            Code::HerdrMissing,
            Code::HerdrUnsupportedVersion,
            Code::HerdrShapeMismatch,
            Code::HerdrVersionUnreadable,
            Code::HerdrUnchecked,
            Code::RegistrationStale,
            Code::RegistrationNoPlace,
            Code::RegistrationPaneGone,
            Code::RegistrationUndecodable,
            Code::PanesUnchecked,
            Code::DeadLettersPresent,
            Code::InflightOverdue,
            Code::BacklogScanTruncated,
            Code::InboxStuck,
            Code::ServiceNotInstalled,
            Code::ServiceInactive,
            Code::ServiceUnknown,
            Code::SocketHeld,
            Code::SocketFallback,
            Code::SocketDeadLettered,
            Code::SocketStatusUnknown,
            Code::SocketWireDrift,
            Code::SocketTranscriptShapeUnrecognised,
        ] {
            assert_eq!(code.name(), pinned(code));
        }
    }

    /// Tracing: TC-029-06
    /// ACs: FR-029-AC-6
    /// Trace: FR-049-AC-1
    #[test]
    fn tc_029_06_the_report_and_check_have_exactly_the_documented_fields() {
        let check = Check::fail(Family::Redis, Code::RedisSlow, Some("x"), "m", "c", "f");
        let report = Report::new(vec![check], false);
        let mut bytes = Vec::new();
        report.write_json(&mut bytes).unwrap();
        assert_eq!(
            bytes,
            b"{\"ok\":false,\"strict\":false,\"checks\":[{\"check\":\"redis\",\"code\":\"redis_slow\",\"status\":\"fail\",\"subject\":\"x\",\"message\":\"m\",\"cause\":\"c\",\"fix\":\"f\",\"details\":[]}]}\n"
        );
        let value = serde_json::to_value(&report).unwrap();
        let keys = |v: &serde_json::Value| {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(keys(&value), ["checks", "ok", "strict"]);
        assert_eq!(
            keys(&value["checks"][0]),
            [
                "cause", "check", "code", "details", "fix", "message", "status", "subject"
            ]
        );
        assert_eq!(value["ok"], false);
        assert_eq!(value["checks"][0]["status"], "fail");
    }

    #[test]
    fn a_check_is_made_safe_at_construction() {
        let noisy = Check::fail(
            Family::Watcher,
            Code::WatcherHeartbeatMissing,
            Some("lu\u{1b}na"),
            "a\nb\u{202e}",
            "c",
            "f",
        )
        .with_details(["x".repeat(1000)]);
        assert!(!noisy.subject.as_deref().unwrap().contains('\u{1b}'));
        assert!(!noisy.message.contains(['\n', '\u{202e}']));
        assert_eq!(noisy.details[0].chars().count(), MAX_LINE_CHARS);
    }
}
