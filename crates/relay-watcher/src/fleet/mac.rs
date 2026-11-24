//! macOS host statistics through the Mach host interface: CPU tick counters and
//! virtual memory counts. The only unsafe code of the collector, each call
//! bounded to a plain C call with a zeroed out-parameter of the documented type.
#![allow(
    unsafe_code,
    deprecated,
    reason = "the Mach host interface is a C API; libc marks mach_host_self deprecated in favour of a crate this workspace does not use"
)]

use super::{
    health::ErrorClass,
    host::{CpuTicks, Memory, ticks_from_macos},
};

/// Cumulative host CPU ticks.
pub fn cpu_ticks() -> Result<CpuTicks, ErrorClass> {
    // SAFETY: `host_cpu_load_info` is plain integers, zero is valid; the call
    // writes at most `count` integers into it and `count` is its size.
    let info = unsafe {
        let mut info: libc::host_cpu_load_info = std::mem::zeroed();
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        let status = libc::host_statistics(
            libc::mach_host_self(),
            libc::HOST_CPU_LOAD_INFO,
            std::ptr::addr_of_mut!(info).cast(),
            &mut count,
        );
        if status != 0 {
            return Err(ErrorClass::Unreadable);
        }
        info
    };
    let tick = |state: libc::c_int| info.cpu_ticks[usize::try_from(state).unwrap_or(0)];
    Ok(ticks_from_macos(
        tick(libc::CPU_STATE_USER),
        tick(libc::CPU_STATE_SYSTEM),
        tick(libc::CPU_STATE_IDLE),
        tick(libc::CPU_STATE_NICE),
    ))
}

/// Used and total memory: total from `hw.memsize`, available as the free and
/// inactive pages of the virtual memory statistics.
pub fn memory() -> Result<Memory, ErrorClass> {
    let mut total: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: `total` and `len` describe the 8-byte buffer `hw.memsize` fills.
    let rc = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            std::ptr::addr_of_mut!(total).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || total == 0 {
        return Err(ErrorClass::Unreadable);
    }
    // SAFETY: as above for `vm_statistics64`; `count` is its size in integers.
    let stats = unsafe {
        let mut stats: libc::vm_statistics64 = std::mem::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let status = libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            std::ptr::addr_of_mut!(stats).cast(),
            &mut count,
        );
        if status != 0 {
            return Err(ErrorClass::Unreadable);
        }
        stats
    };
    // SAFETY: `sysconf` takes a constant and returns a number.
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) })
        .map_err(|_| ErrorClass::Unreadable)?;
    let available =
        (u64::from(stats.free_count) + u64::from(stats.inactive_count)).saturating_mul(page);
    Ok(Memory {
        used: total.saturating_sub(available),
        total,
    })
}

/// Process names and working directories: `(name, cwd)` of every process
/// whose name `keep` accepts.
pub fn processes(keep: &dyn Fn(&str) -> bool) -> Result<Vec<(String, String)>, ErrorClass> {
    // SAFETY: a null buffer asks for the byte size needed.
    let bytes = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if bytes <= 0 {
        return Err(ErrorClass::Unreadable);
    }
    let capacity = usize::try_from(bytes).unwrap_or(0) / std::mem::size_of::<libc::c_int>() + 64;
    let mut pids: Vec<libc::c_int> = vec![0; capacity];
    let size = libc::c_int::try_from(capacity * std::mem::size_of::<libc::c_int>())
        .map_err(|_| ErrorClass::Unreadable)?;
    // SAFETY: `pids` holds `size` bytes of writable integers.
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size) };
    if got <= 0 {
        return Err(ErrorClass::Unreadable);
    }
    let count = usize::try_from(got).unwrap_or(0) / std::mem::size_of::<libc::c_int>();
    let mut out = Vec::new();
    for pid in pids.into_iter().take(count).filter(|p| *p > 0) {
        let mut name = [0u8; 256];
        // SAFETY: the buffer is 256 writable bytes.
        let n = unsafe { libc::proc_name(pid, name.as_mut_ptr().cast(), 256) };
        let Ok(n) = usize::try_from(n) else { continue };
        let name = String::from_utf8_lossy(&name[..n.min(256)]).into_owned();
        if n == 0 || !keep(&name) {
            continue;
        }
        // SAFETY: `proc_vnodepathinfo` is plain data, zero is valid; the call
        // fills at most its size.
        let cwd = unsafe {
            let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
            let want =
                libc::c_int::try_from(std::mem::size_of::<libc::proc_vnodepathinfo>()).unwrap_or(0);
            let r = libc::proc_pidinfo(
                pid,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                std::ptr::addr_of_mut!(info).cast(),
                want,
            );
            if r < want {
                continue;
            }
            let raw = info.pvi_cdir.vip_path;
            let bytes: Vec<u8> = raw
                .iter()
                .flatten()
                .map(|c| *c as u8)
                .take_while(|b| *b != 0)
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        };
        out.push((name, cwd));
    }
    Ok(out)
}
