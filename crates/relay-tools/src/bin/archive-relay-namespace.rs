//! `archive-relay-namespace --prefix <old prefix> --output <file>`
//!
//! Archives the keys of a prior relay namespace without modifying them. The
//! Redis URL resolves as in `agent-msg`: `IX_RELAY_URL`, else `url` in the
//! config file (`IX_RELAY_CONFIG`, default `~/.ix/relay/config.toml`), else
//! the default local URL.

use std::{path::PathBuf, process::ExitCode};

use relay_core::config::{CoreFlags, load_core};

const FAILED: &str =
    "archive failed; source keys unchanged; inspect any partial output before retrying";

fn parse() -> Option<(String, PathBuf)> {
    let (mut prefix, mut output) = (None, None);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (flag, None),
        };
        let value = inline.or_else(|| args.next())?;
        match name.as_str() {
            "--prefix" => prefix = Some(value),
            "--output" => output = Some(PathBuf::from(value)),
            _ => return None,
        }
    }
    Some((prefix?, output?))
}

fn main() -> ExitCode {
    let Some((prefix, output)) = parse() else {
        eprintln!("usage: archive-relay-namespace --prefix <prefix> --output <file>");
        return ExitCode::from(2);
    };
    let settings = match load_core(CoreFlags::default()) {
        Ok(settings) => settings,
        Err(err) => {
            eprintln!("{err}");
            eprintln!("{FAILED}");
            return ExitCode::FAILURE;
        }
    };
    match relay_tools::archive::archive(&settings.url.value, &prefix, &output) {
        Ok(count) => {
            println!("archived {count} keys; source keys unchanged");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            eprintln!("{FAILED}");
            ExitCode::FAILURE
        }
    }
}
