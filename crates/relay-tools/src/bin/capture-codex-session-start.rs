//! Codex `SessionStart` command hook; reads the payload on stdin.

use std::path::PathBuf;

fn main() {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")));
    if let Some(home) = home {
        relay_tools::session_start::run(std::io::stdin(), &home, |key| std::env::var(key).ok());
    }
}
