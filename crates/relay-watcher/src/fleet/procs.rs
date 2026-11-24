//! Process lists (FR-069): compiler and build driver processes with their
//! working directories, and native Windows agent processes seen from WSL.

use std::{fs, path::Path};

use super::health::ErrorClass;

/// The default compilers and build drivers.
pub const DEFAULT_COMPILERS: &[&str] = &["rustc", "cargo", "cc", "clang", "gcc", "ld", "tsc"];

/// Image names that must not run natively on a WSL host: an agent, herdr or a
/// build process on the Windows side is a defect.
pub const NATIVE_WINDOWS_IMAGES: &[&str] = &[
    "claude.exe",
    "codex.exe",
    "herdr.exe",
    "cargo.exe",
    "rustc.exe",
];

/// A compiler process: its executable name and working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiler {
    /// Executable name.
    pub name: String,
    /// Working directory.
    pub cwd: String,
}

#[cfg(any(target_os = "linux", test))]
/// The compiler processes under a `/proc`-shaped `root`: each numeric
/// directory's `comm` is matched against `names`, and `cwd` read as a link. A
/// process that exits mid-read is skipped, as it is no longer a process.
///
/// # Errors
///
/// [`ErrorClass::Unreadable`] when `root` cannot be listed.
pub fn compilers_in_proc(root: &Path, names: &[&str]) -> Result<Vec<Compiler>, ErrorClass> {
    let mut found = Vec::new();
    for entry in fs::read_dir(root)
        .map_err(|_| ErrorClass::Unreadable)?
        .flatten()
    {
        let file_name = entry.file_name();
        if !file_name
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let Ok(comm) = fs::read_to_string(entry.path().join("comm")) else {
            continue;
        };
        let name = comm.trim();
        if !names.contains(&name) {
            continue;
        }
        let Ok(cwd) = fs::read_link(entry.path().join("cwd")) else {
            continue;
        };
        found.push(Compiler {
            name: name.to_owned(),
            cwd: cwd.to_string_lossy().into_owned(),
        });
    }
    Ok(found)
}

/// The compiler processes of this host.
///
/// # Errors
///
/// An [`ErrorClass`] when the process list cannot be read.
pub fn read_compilers(names: &[&str]) -> Result<Vec<Compiler>, ErrorClass> {
    #[cfg(target_os = "linux")]
    {
        compilers_in_proc(Path::new("/proc"), names)
    }
    #[cfg(target_os = "macos")]
    {
        let rows = super::mac::processes(&|n| names.contains(&n))?;
        Ok(rows
            .into_iter()
            .map(|(name, cwd)| Compiler { name, cwd })
            .collect())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = names;
        Err(ErrorClass::Unsupported)
    }
}

/// How many processes of a Windows `tasklist /FO CSV /NH` listing are a
/// native agent, herdr or build process.
#[must_use]
pub fn count_native_windows(tasklist_csv: &str) -> usize {
    tasklist_csv
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"')?.split('"').next())
        .filter(|image| {
            NATIVE_WINDOWS_IMAGES
                .iter()
                .any(|n| image.eq_ignore_ascii_case(n))
        })
        .count()
}

/// Whether this is a WSL guest (the interop socket or a Microsoft kernel).
#[must_use]
pub fn is_wsl() -> bool {
    Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists()
        || fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|r| r.to_ascii_lowercase().contains("microsoft"))
}

/// Native Windows agent processes seen through the WSL interop process list.
///
/// # Errors
///
/// An [`ErrorClass`] when the list cannot be read in time.
pub fn read_native_windows() -> Result<usize, ErrorClass> {
    use herdr_cli::{ProcessError, ProcessRunner, RealProcessRunner};
    let out = RealProcessRunner
        .run(
            "tasklist.exe",
            &["/FO", "CSV", "/NH"],
            std::time::Duration::from_millis(400),
        )
        .map_err(|e| match e {
            ProcessError::Timeout { .. } => ErrorClass::Timeout,
            // No `tasklist.exe`: WSL interop is off or this is no WSL guest.
            ProcessError::Spawn { .. } => ErrorClass::Unsupported,
            _ => ErrorClass::Unavailable,
        })?;
    if !out.success {
        return Err(ErrorClass::Unavailable);
    }
    Ok(count_native_windows(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ix_trace_rs::trace;

    #[trace("FR-069-AC-14")]
    #[test]
    fn tc_069_14_a_native_windows_agent_in_the_interop_list_is_counted() {
        let with = "\"System\",\"4\",\"Services\",\"0\",\"8 K\"\r\n\
                    \"claude.exe\",\"123\",\"Console\",\"1\",\"90 K\"\r\n\
                    \"Codex.EXE\",\"124\",\"Console\",\"1\",\"90 K\"\r\n\
                    \"chrome.exe\",\"5\",\"Console\",\"1\",\"1 K\"\r\n";
        assert_eq!(count_native_windows(with), 2);
        let without = "\"System\",\"4\",\"Services\",\"0\",\"8 K\"\r\n";
        assert_eq!(count_native_windows(without), 0);
        assert_eq!(count_native_windows(""), 0);
    }

    #[test]
    fn compilers_are_listed_from_a_proc_tree_with_their_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work/qsl-1");
        fs::create_dir_all(&work).unwrap();
        for (pid, comm) in [("100", "rustc"), ("101", "bash"), ("102", "cargo")] {
            let p = dir.path().join("proc").join(pid);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("comm"), format!("{comm}\n")).unwrap();
            std::os::unix::fs::symlink(&work, p.join("cwd")).unwrap();
        }
        fs::create_dir_all(dir.path().join("proc/self")).unwrap();
        let mut found = compilers_in_proc(&dir.path().join("proc"), DEFAULT_COMPILERS).unwrap();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "cargo");
        assert_eq!(found[1].cwd, work.to_string_lossy());
        assert_eq!(
            compilers_in_proc(&dir.path().join("absent"), DEFAULT_COMPILERS),
            Err(ErrorClass::Unreadable)
        );
    }
}
