//! Linux pressure stall information (PSI): how long tasks stalled waiting for
//! CPU, IO or memory, read from `/proc/pressure/{cpu,io,memory}`. A host busy on
//! IO shows low CPU pressure and high IO pressure, which the CPU busy ratio
//! cannot show. Values are percent of wall time. Only Linux has PSI; elsewhere
//! (and on a kernel built without it) the reading is
//! [`ErrorClass::Unsupported`], which callers report as unavailable, not failed.

#[cfg(any(target_os = "linux", test, feature = "fixtures"))]
use std::{fs, io::ErrorKind, path::Path};

use crate::ErrorClass;

/// The resource a pressure reading is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resource {
    /// Runnable tasks waiting for a CPU.
    Cpu,
    /// Tasks waiting for block IO.
    Io,
    /// Tasks waiting for memory (reclaim, swap).
    Memory,
}

impl Resource {
    /// Every resource.
    pub const ALL: [Resource; 3] = [Resource::Cpu, Resource::Io, Resource::Memory];

    /// The resource as named in `/proc/pressure/<name>` and in metric labels.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Resource::Cpu => "cpu",
            Resource::Io => "io",
            Resource::Memory => "memory",
        }
    }
}

/// Stall percentages over the last 10 and 60 seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Averages {
    /// Percent of the last 10 seconds.
    pub avg10: f64,
    /// Percent of the last 60 seconds.
    pub avg60: f64,
}

/// One resource's pressure: `some` (at least one task stalled) and `full` (all
/// non-idle tasks stalled). Older kernels give no `full` line for CPU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pressure {
    /// At least one task stalled.
    pub some: Averages,
    /// Every non-idle task stalled, where the kernel reports it.
    pub full: Option<Averages>,
}

/// The pressure of all three resources, read together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressureSet {
    /// CPU pressure.
    pub cpu: Pressure,
    /// IO pressure.
    pub io: Pressure,
    /// Memory pressure.
    pub memory: Pressure,
}

impl PressureSet {
    /// The reading of `resource`.
    #[must_use]
    pub fn get(&self, resource: Resource) -> Pressure {
        match resource {
            Resource::Cpu => self.cpu,
            Resource::Io => self.io,
            Resource::Memory => self.memory,
        }
    }
}

#[cfg(any(target_os = "linux", test, feature = "fixtures"))]
/// Parses one `/proc/pressure/<resource>` file.
///
/// # Errors
///
/// [`ErrorClass::Parse`] when the `some` line is missing or a figure is not a
/// number.
pub fn parse_pressure(text: &str) -> Result<Pressure, ErrorClass> {
    let mut some = None;
    let mut full = None;
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let slot = match words.next() {
            Some("some") => &mut some,
            Some("full") => &mut full,
            _ => continue,
        };
        let (mut avg10, mut avg60) = (None, None);
        for word in words {
            let Some((key, value)) = word.split_once('=') else {
                continue;
            };
            let target = match key {
                "avg10" => &mut avg10,
                "avg60" => &mut avg60,
                _ => continue,
            };
            let n: f64 = value.parse().map_err(|_| ErrorClass::Parse)?;
            if !n.is_finite() {
                return Err(ErrorClass::Parse);
            }
            *target = Some(n);
        }
        *slot = Some(Averages {
            avg10: avg10.ok_or(ErrorClass::Parse)?,
            avg60: avg60.ok_or(ErrorClass::Parse)?,
        });
    }
    Ok(Pressure {
        some: some.ok_or(ErrorClass::Parse)?,
        full,
    })
}

#[cfg(any(target_os = "linux", test, feature = "fixtures"))]
/// Reads the pressure of every resource under a `/proc/pressure`-shaped `root`.
///
/// # Errors
///
/// [`ErrorClass::Unsupported`] when a file is absent (a kernel built without PSI)
/// or the read is refused as unsupported (PSI disabled at boot),
/// [`ErrorClass::Unreadable`] for any other read failure,
/// [`ErrorClass::Parse`] for malformed text.
pub fn read_pressure_at(root: &Path) -> Result<PressureSet, ErrorClass> {
    let one = |resource: Resource| -> Result<Pressure, ErrorClass> {
        let text = fs::read_to_string(root.join(resource.name())).map_err(|e| read_failure(&e))?;
        parse_pressure(&text)
    };
    Ok(PressureSet {
        cpu: one(Resource::Cpu)?,
        io: one(Resource::Io)?,
        memory: one(Resource::Memory)?,
    })
}

#[cfg(any(target_os = "linux", test, feature = "fixtures"))]
/// A missing file (no PSI in the kernel) and `EOPNOTSUPP` (PSI disabled at boot
/// with `psi=0`) mean the host has no pressure readings; any other failure is a
/// failed read.
fn read_failure(e: &std::io::Error) -> ErrorClass {
    match e.kind() {
        ErrorKind::NotFound | ErrorKind::Unsupported => ErrorClass::Unsupported,
        _ => ErrorClass::Unreadable,
    }
}

/// The pressure of this host.
///
/// # Errors
///
/// [`ErrorClass::Unsupported`] off Linux or on a kernel without PSI, otherwise
/// an [`ErrorClass`] for the failed read.
pub fn read_pressure() -> Result<PressureSet, ErrorClass> {
    #[cfg(target_os = "linux")]
    {
        read_pressure_at(Path::new("/proc/pressure"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(ErrorClass::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ix_trace_rs::trace;

    const IO: &str = "some avg10=28.50 avg60=20.25 avg300=9.00 total=123456\n\
                      full avg10=21.00 avg60=15.50 avg300=6.00 total=98765\n";
    // Older kernels report CPU `some` only.
    const CPU: &str = "some avg10=1.50 avg60=0.75 avg300=0.10 total=42\n";
    const MEM: &str = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n\
                       full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";

    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            fs::write(dir.path().join(name), text).unwrap();
        }
        dir
    }

    #[trace("FR-069-AC-17")]
    #[test]
    fn tc_069_17_proc_pressure_fixtures_give_some_and_full_avg10_and_avg60_per_resource() {
        let dir = fixture(&[("cpu", CPU), ("io", IO), ("memory", MEM)]);
        let set = read_pressure_at(dir.path()).unwrap();
        assert_eq!(set.io.some.avg10, 28.5);
        assert_eq!(set.io.some.avg60, 20.25);
        assert_eq!(set.io.full.unwrap().avg10, 21.0);
        assert_eq!(set.io.full.unwrap().avg60, 15.5);
        assert_eq!(set.cpu.some.avg10, 1.5);
        assert_eq!(set.cpu.full, None, "no full line on this kernel");
        assert_eq!(set.get(Resource::Memory).some.avg60, 0.0);
    }

    #[trace("FR-069-AC-21")]
    #[test]
    fn tc_069_21_eopnotsupp_from_a_psi_disabled_boot_is_unsupported_and_other_errors_unreadable() {
        let refused = std::io::Error::from(ErrorKind::Unsupported);
        assert_eq!(read_failure(&refused), ErrorClass::Unsupported);
        let denied = std::io::Error::from(ErrorKind::PermissionDenied);
        assert_eq!(read_failure(&denied), ErrorClass::Unreadable);
        // A directory in place of the file is a failed read, not an absent one.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("cpu")).unwrap();
        assert_eq!(read_pressure_at(dir.path()), Err(ErrorClass::Unreadable));
    }
    #[trace("FR-069-AC-18")]
    #[test]
    fn tc_069_18_a_host_without_proc_pressure_is_unsupported_and_garbage_is_a_parse_failure() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(read_pressure_at(empty.path()), Err(ErrorClass::Unsupported));
        let partial = fixture(&[("cpu", CPU), ("io", IO)]);
        assert_eq!(
            read_pressure_at(partial.path()),
            Err(ErrorClass::Unsupported)
        );
        assert_eq!(
            parse_pressure("full avg10=1 avg60=1\n"),
            Err(ErrorClass::Parse)
        );
        assert_eq!(
            parse_pressure("some avg10=x avg60=1\n"),
            Err(ErrorClass::Parse)
        );
        assert_eq!(parse_pressure("some avg10=1\n"), Err(ErrorClass::Parse));
        #[cfg(not(target_os = "linux"))]
        assert_eq!(read_pressure(), Err(ErrorClass::Unsupported));
    }
}
