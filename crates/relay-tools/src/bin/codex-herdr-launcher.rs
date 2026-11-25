//! Runs the real `codex` found on `PATH` (never itself), adding `--no-daemon`
//! to interactive Herdr starts.

use std::{
    ffi::OsString,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use rustix::fs::{Access, access};

fn is_executable_file(path: &Path) -> bool {
    path.is_file() && access(path, Access::EXEC_OK).is_ok()
}

fn main() -> ExitCode {
    let own = std::env::current_exe().and_then(std::fs::canonicalize).ok();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let in_herdr = std::env::var_os("HERDR_ENV").is_some_and(|v| v == "1");
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    for directory in std::env::split_paths(&path) {
        let candidate: PathBuf = directory.join("codex");
        if is_executable_file(&candidate) && std::fs::canonicalize(&candidate).ok() != own {
            let err = Command::new(&candidate)
                .args(relay_tools::launcher::arguments(&args, in_herdr))
                .exec();
            eprintln!("codex: cannot run {}: {err}", candidate.display());
            return ExitCode::FAILURE;
        }
    }
    eprintln!("codex: the Herdr launcher could not find the underlying Codex executable on PATH");
    ExitCode::FAILURE
}
