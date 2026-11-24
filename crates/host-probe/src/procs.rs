//! Compiler and build driver processes with their working directories (FR-069).

#[cfg(any(target_os = "linux", test))]
use std::{fs, path::Path};

use crate::ErrorClass;

/// The default compilers and build drivers.
pub const DEFAULT_COMPILERS: &[&str] = &["rustc", "cargo", "cc", "clang", "gcc", "ld", "tsc"];

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
        let rows = crate::mac::processes(&|n| names.contains(&n))?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
