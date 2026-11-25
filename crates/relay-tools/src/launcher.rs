//! Keeps interactive Herdr Codex sessions in their own terminal context by
//! adding `--no-daemon`, so a shared Codex daemon cannot supply another
//! terminal's stale environment.

use std::ffi::OsString;

/// Subcommands that are plain CLI commands, never interactive sessions.
const NONINTERACTIVE: &[&str] = &[
    "exec",
    "e",
    "review",
    "login",
    "logout",
    "mcp",
    "mcp-server",
    "app-server",
    "app-server-daemon",
    "completion",
    "sandbox",
    "debug",
    "apply",
    "a",
    "cloud",
    "features",
    "help",
    "version",
    "update",
];

/// Options whose next argument is a value, not a subcommand.
const VALUE_OPTIONS: &[&str] = &[
    "--config",
    "-c",
    "--model",
    "-m",
    "--profile",
    "-p",
    "--sandbox",
    "-s",
    "--ask-for-approval",
    "-a",
    "--cd",
    "-C",
    "--image",
    "-i",
    "--add-dir",
    "--enable",
    "--disable",
];

/// The arguments to hand the real Codex. Only interactive starts, resumes and
/// forks inside Herdr (`in_herdr`, that is `HERDR_ENV=1`) gain `--no-daemon`.
pub fn arguments(args: &[OsString], in_herdr: bool) -> Vec<OsString> {
    let text = |arg: &OsString| arg.to_str().map(str::to_owned);
    let mut first: Option<String> = None;
    let mut skip_value = false;
    // Skip option values before deciding whether the first positional token is
    // a subcommand: a model or profile named "exec" is still interactive.
    for arg in args.iter().filter_map(text) {
        if skip_value {
            skip_value = false;
            continue;
        }
        if arg == "--" {
            break;
        }
        if VALUE_OPTIONS.contains(&arg.as_str()) {
            skip_value = true;
        } else if !arg.starts_with('-') {
            first = Some(arg);
            break;
        }
    }
    let has = |flags: &[&str]| {
        args.iter()
            .filter_map(text)
            .any(|a| flags.contains(&a.as_str()))
    };
    let informational = has(&["--help", "-h", "--version", "-V"]);
    let noninteractive = first
        .as_deref()
        .is_some_and(|f| NONINTERACTIVE.contains(&f));
    if in_herdr && !informational && !noninteractive && !has(&["--no-daemon"]) {
        let mut out = vec![OsString::from("--no-daemon")];
        out.extend_from_slice(args);
        return out;
    }
    args.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn interactive_starts_resumes_and_forks_gain_no_daemon() {
        for args in [
            vec![],
            vec!["resume", "session-id"],
            vec!["fork", "--last"],
            vec!["--model", "gpt-6", "a prompt"],
            vec!["--model", "exec"],
            vec!["--config", "x=exec", "resume", "session-id"],
        ] {
            let mut want = os(&["--no-daemon"]);
            want.extend(os(&args));
            assert_eq!(arguments(&os(&args), true), want, "{args:?}");
        }
    }

    #[test]
    fn noninteractive_and_outside_herdr_pass_through() {
        for args in [
            vec!["--model", "gpt-6", "exec", "prompt"],
            vec!["exec", "prompt"],
            vec!["--version"],
            vec!["mcp", "list"],
            vec!["resume", "--no-daemon", "session"],
        ] {
            assert_eq!(arguments(&os(&args), true), os(&args), "{args:?}");
        }
        assert_eq!(
            arguments(&os(&["resume", "session"]), false),
            os(&["resume", "session"])
        );
    }
}
