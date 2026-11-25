//! Native Windows agent processes seen from WSL (FR-069).

use std::{fs, path::Path};

use host_probe::ErrorClass;

/// Image names that must not run natively on a WSL host: an agent, herdr or a
/// build process on the Windows side is a defect.
pub const NATIVE_WINDOWS_IMAGES: &[&str] = &[
    "claude.exe",
    "codex.exe",
    "herdr.exe",
    "cargo.exe",
    "rustc.exe",
];

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
}
