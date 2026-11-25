//! `archive-relay-namespace --prefix <old prefix> --output <file> [--config <toml>]`
//!
//! Archives the keys of a prior relay namespace without modifying them. The
//! Redis URL is `IX_RELAY_URL`, else `url` in the config file
//! (default `~/.ix/relay/config.toml`).

use std::{path::PathBuf, process::ExitCode};

const FAILED: &str =
    "archive failed; source keys unchanged; inspect any partial output before retrying";

struct Args {
    prefix: String,
    output: PathBuf,
    config: PathBuf,
}

fn parse() -> Option<Args> {
    let (mut prefix, mut output, mut config) = (None, None, None);
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
            "--config" => config = Some(PathBuf::from(value)),
            _ => return None,
        }
    }
    let config = config.or_else(|| {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".ix/relay/config.toml"))
    })?;
    Some(Args {
        prefix: prefix?,
        output: output?,
        config,
    })
}

fn url(config: &std::path::Path) -> Option<String> {
    match std::env::var("IX_RELAY_URL") {
        Ok(url) if !url.is_empty() => Some(url),
        _ => {
            let table: toml::Table = std::fs::read_to_string(config).ok()?.parse().ok()?;
            table.get("url")?.as_str().map(str::to_owned)
        }
    }
}

fn main() -> ExitCode {
    let Some(args) = parse() else {
        eprintln!(
            "usage: archive-relay-namespace --prefix <prefix> --output <file> [--config <file>]"
        );
        return ExitCode::from(2);
    };
    let archived = url(&args.config).ok_or(()).and_then(|url| {
        relay_tools::archive::archive(&url, &args.prefix, &args.output).map_err(|_| ())
    });
    match archived {
        Ok(count) => {
            println!("archived {count} keys; source keys unchanged");
            ExitCode::SUCCESS
        }
        Err(()) => {
            eprintln!("{FAILED}");
            ExitCode::FAILURE
        }
    }
}
