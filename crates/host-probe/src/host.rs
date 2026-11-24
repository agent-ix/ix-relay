//! Host measurements (FR-069): CPU busy ratio, memory and temperature, read
//! natively. Linux and WSL read `/proc` and `/sys`; macOS reads the host
//! statistics interface. The platform is chosen by build target, and nothing
//! here spawns a process.

use std::time::{Duration, Instant};
#[cfg(any(target_os = "linux", test, feature = "fixtures"))]
use std::{fs, path::Path};

use crate::ErrorClass;

/// The shortest interval between two CPU samples that yields a ratio.
pub const MIN_CPU_INTERVAL: Duration = Duration::from_secs(1);

/// Cumulative CPU time in ticks: busy and idle (iowait counts as idle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTicks {
    /// Non-idle ticks.
    pub busy: u64,
    /// Idle plus iowait ticks.
    pub idle: u64,
}

#[cfg(any(target_os = "linux", test, feature = "fixtures"))]
/// Parses the aggregate `cpu` line of `/proc/stat`.
///
/// # Errors
///
/// [`ErrorClass::Parse`] when the line is missing or short.
pub fn parse_proc_stat(text: &str) -> Result<CpuTicks, ErrorClass> {
    let line = text
        .lines()
        .find(|l| l.split_whitespace().next() == Some("cpu"))
        .ok_or(ErrorClass::Parse)?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(|f| f.parse().map_err(|_| ErrorClass::Parse))
        .collect::<Result<_, _>>()?;
    // user nice system idle iowait irq softirq steal
    let [user, nice, system, idle, iowait, irq, softirq, steal] = fields[..] else {
        return Err(ErrorClass::Parse);
    };
    let idle_all = idle.saturating_add(iowait);
    let busy = [user, nice, system, irq, softirq, steal]
        .into_iter()
        .fold(0u64, u64::saturating_add);
    Ok(CpuTicks {
        busy,
        idle: idle_all,
    })
}

/// The ticks of the macOS host CPU counters: user, system and nice are busy.
#[cfg(any(target_os = "macos", test, feature = "fixtures"))]
#[must_use]
pub fn ticks_from_macos(user: u32, system: u32, idle: u32, nice: u32) -> CpuTicks {
    CpuTicks {
        busy: u64::from(user) + u64::from(system) + u64::from(nice),
        idle: u64::from(idle),
    }
}

/// Turns successive [`CpuTicks`] into a busy ratio: the non-idle share of the
/// time between two samples at least [`MIN_CPU_INTERVAL`] apart. One sample
/// never gives a ratio.
#[derive(Debug, Default)]
pub struct CpuSampler {
    previous: Option<(Instant, CpuTicks)>,
}

impl CpuSampler {
    /// Offers a sample taken at `at`. `None` for the first sample, for one less
    /// than a second after the last kept one (that one stays the baseline), and
    /// for counters that did not advance or went backwards (a new baseline).
    pub fn sample(&mut self, at: Instant, ticks: CpuTicks) -> Option<f64> {
        let Some((then, before)) = self.previous else {
            self.previous = Some((at, ticks));
            return None;
        };
        if at.saturating_duration_since(then) < MIN_CPU_INTERVAL {
            return None;
        }
        self.previous = Some((at, ticks));
        let busy = ticks.busy.checked_sub(before.busy)?;
        let idle = ticks.idle.checked_sub(before.idle)?;
        let total = busy.checked_add(idle).filter(|t| *t > 0)?;
        #[expect(clippy::cast_precision_loss, reason = "tick deltas are far below 2^52")]
        Some(busy as f64 / total as f64)
    }
}

/// Memory in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    /// Total minus available.
    pub used: u64,
    /// Total.
    pub total: u64,
}

#[cfg(any(target_os = "linux", test, feature = "fixtures"))]
/// Parses `/proc/meminfo`: used is `MemTotal` minus `MemAvailable`.
///
/// # Errors
///
/// [`ErrorClass::Parse`] when either field is missing or not a number.
pub fn parse_meminfo(text: &str) -> Result<Memory, ErrorClass> {
    let field = |name: &str| -> Result<u64, ErrorClass> {
        let line = text
            .lines()
            .find(|l| l.starts_with(name) && l[name.len()..].starts_with(':'))
            .ok_or(ErrorClass::Parse)?;
        let kib: u64 = line[name.len() + 1..]
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or(ErrorClass::Parse)?;
        kib.checked_mul(1024).ok_or(ErrorClass::Parse)
    };
    let total = field("MemTotal")?;
    let available = field("MemAvailable")?;
    Ok(Memory {
        used: total.saturating_sub(available),
        total,
    })
}

#[cfg(any(target_os = "linux", test, feature = "fixtures"))]
/// The highest `Tctl` or CPU package temperature under a hwmon root
/// (`/sys/class/hwmon`), in degrees Celsius.
///
/// # Errors
///
/// [`ErrorClass::Unreadable`] when the root cannot be read or holds no such
/// sensor, [`ErrorClass::Parse`] when a candidate holds no number.
pub fn read_hwmon(root: &Path) -> Result<f64, ErrorClass> {
    let mut best: Option<f64> = None;
    for chip in fs::read_dir(root)
        .map_err(|_| ErrorClass::Unreadable)?
        .flatten()
    {
        let Ok(files) = fs::read_dir(chip.path()) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            let Some(stem) = name
                .strip_prefix("temp")
                .and_then(|r| r.strip_suffix("_label"))
            else {
                continue;
            };
            let Ok(label) = fs::read_to_string(file.path()) else {
                continue;
            };
            let label = label.trim();
            if !(label == "Tctl" || label == "Tdie" || label.starts_with("Package id")) {
                continue;
            }
            let input = chip.path().join(format!("temp{stem}_input"));
            let milli: f64 = fs::read_to_string(input)
                .map_err(|_| ErrorClass::Unreadable)?
                .trim()
                .parse()
                .map_err(|_| ErrorClass::Parse)?;
            let celsius = milli / 1000.0;
            best = Some(best.map_or(celsius, |b: f64| b.max(celsius)));
        }
    }
    best.ok_or(ErrorClass::Unreadable)
}

/// Cumulative CPU ticks of this host.
///
/// # Errors
///
/// An [`ErrorClass`] when the platform cannot be read.
pub fn read_cpu_ticks() -> Result<CpuTicks, ErrorClass> {
    #[cfg(target_os = "linux")]
    {
        let text = fs::read_to_string("/proc/stat").map_err(|_| ErrorClass::Unreadable)?;
        parse_proc_stat(&text)
    }
    #[cfg(target_os = "macos")]
    {
        crate::mac::cpu_ticks()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(ErrorClass::Unsupported)
    }
}

/// Memory of this host.
///
/// # Errors
///
/// An [`ErrorClass`] when the platform cannot be read.
pub fn read_memory() -> Result<Memory, ErrorClass> {
    #[cfg(target_os = "linux")]
    {
        let text = fs::read_to_string("/proc/meminfo").map_err(|_| ErrorClass::Unreadable)?;
        parse_meminfo(&text)
    }
    #[cfg(target_os = "macos")]
    {
        crate::mac::memory()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(ErrorClass::Unsupported)
    }
}

/// The CPU temperature of this host.
///
/// # Errors
///
/// An [`ErrorClass`] when the host has no readable sensor.
pub fn read_temperature() -> Result<f64, ErrorClass> {
    #[cfg(target_os = "linux")]
    {
        read_hwmon(Path::new("/sys/class/hwmon"))
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

    const STAT_A: &str = "cpu  1000 0 500 8000 500 0 0 0 0 0\ncpu0 1000 0 500 8000 500 0 0 0 0 0\n";
    // +300 user, +100 system, +500 idle, +100 iowait over the interval.
    const STAT_B: &str = "cpu  1300 0 600 8500 600 0 0 0 0 0\n";

    fn at(secs: u64) -> Instant {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_secs(secs)
    }

    #[trace("FR-069-AC-1")]
    #[test]
    fn tc_069_01_two_proc_stat_samples_give_the_non_idle_share_and_one_gives_none() {
        let mut s = CpuSampler::default();
        assert_eq!(s.sample(at(0), parse_proc_stat(STAT_A).unwrap()), None);
        // busy delta 400, idle (idle+iowait) delta 600: 400 / 1000.
        let ratio = s.sample(at(1), parse_proc_stat(STAT_B).unwrap()).unwrap();
        assert!((ratio - 0.4).abs() < 1e-9, "{ratio}");
        // Less than a second after the baseline: no ratio, baseline kept.
        let mut s = CpuSampler::default();
        s.sample(at(10), parse_proc_stat(STAT_A).unwrap());
        assert_eq!(s.sample(at(10), parse_proc_stat(STAT_B).unwrap()), None);
        assert!(s.sample(at(11), parse_proc_stat(STAT_B).unwrap()).is_some());
    }

    #[trace("FR-069-AC-2")]
    #[test]
    fn tc_069_02_macos_host_tick_fixtures_give_the_expected_ratio_on_the_build_target_path() {
        let mut s = CpuSampler::default();
        s.sample(at(0), ticks_from_macos(1000, 500, 8000, 100));
        // +200 user, +100 system, +100 nice busy; +600 idle: 400 / 1000.
        let ratio = s
            .sample(at(2), ticks_from_macos(1200, 600, 8600, 200))
            .unwrap();
        assert!((ratio - 0.4).abs() < 1e-9, "{ratio}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_live_macos_host_reads_cpu_memory_and_processes() {
        let ticks = read_cpu_ticks().unwrap();
        assert!(ticks.busy + ticks.idle > 0);
        let m = read_memory().unwrap();
        assert!(m.total > 0 && m.used <= m.total);
        assert!(!crate::mac::processes(&|_| true).unwrap().is_empty());
    }

    #[test]
    fn counters_that_did_not_advance_or_went_backwards_give_no_ratio() {
        let mut s = CpuSampler::default();
        let a = parse_proc_stat(STAT_B).unwrap();
        s.sample(at(0), a);
        assert_eq!(s.sample(at(2), a), None, "no ticks elapsed");
        assert_eq!(
            s.sample(at(4), parse_proc_stat(STAT_A).unwrap()),
            None,
            "went back"
        );
    }

    #[test]
    fn a_short_or_missing_cpu_line_is_a_parse_failure() {
        assert_eq!(parse_proc_stat("cpu 1 2 3\n"), Err(ErrorClass::Parse));
        assert_eq!(parse_proc_stat("intr 1\n"), Err(ErrorClass::Parse));
        assert_eq!(
            parse_proc_stat("cpu a b c d e f g h\n"),
            Err(ErrorClass::Parse)
        );
    }

    #[trace("FR-069-AC-3")]
    #[test]
    fn tc_069_03_a_wsl_proc_fixture_gives_cpu_and_memory_from_text_alone() {
        // The WSL VM exposes the Linux /proc files; the parsers take text, so no
        // process is spawned and no ssh session is involved.
        let meminfo =
            "MemTotal:       16384000 kB\nMemFree:  100 kB\nMemAvailable:    4096000 kB\n";
        let m = parse_meminfo(meminfo).unwrap();
        assert_eq!(m.total, 16_384_000 * 1024);
        assert_eq!(m.used, (16_384_000 - 4_096_000) * 1024);
        assert!(parse_proc_stat(STAT_A).is_ok());
        assert_eq!(parse_meminfo("MemTotal: 1 kB\n"), Err(ErrorClass::Parse));
    }

    #[trace("FR-069-AC-4")]
    #[test]
    fn tc_069_04_the_highest_tctl_or_package_sensor_is_read_and_a_missing_one_fails() {
        let dir = tempfile::tempdir().unwrap();
        let chip = dir.path().join("hwmon0");
        fs::create_dir(&chip).unwrap();
        fs::write(chip.join("temp1_label"), "Tctl\n").unwrap();
        fs::write(chip.join("temp1_input"), "61250\n").unwrap();
        fs::write(chip.join("temp2_label"), "Package id 0\n").unwrap();
        fs::write(chip.join("temp2_input"), "70000\n").unwrap();
        fs::write(chip.join("temp3_label"), "Core 3\n").unwrap();
        fs::write(chip.join("temp3_input"), "99000\n").unwrap();
        assert!((read_hwmon(dir.path()).unwrap() - 70.0).abs() < 1e-9);
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(read_hwmon(empty.path()), Err(ErrorClass::Unreadable));
        assert_eq!(
            read_hwmon(&dir.path().join("absent")),
            Err(ErrorClass::Unreadable)
        );
    }
}
