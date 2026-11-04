//! Resident memory of the workload process, not the renderer or machine total.

#[cfg(target_os = "macos")]
pub(crate) fn resident_bytes(pid: u32) -> Option<u64> {
    let pid = i32::try_from(pid).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_taskinfo>();
    // SAFETY: the buffer is valid for `size` bytes and is read only after a
    // complete successful response. The OS accepts the workload process ID.
    let written = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, info.as_mut_ptr().cast(), i32::try_from(size).ok()?) };
    if usize::try_from(written).ok()? != size {
        return None;
    }
    // SAFETY: proc_pidinfo reported that it initialized the whole structure.
    Some(unsafe { info.assume_init() }.pti_resident_size)
}

#[cfg(target_os = "linux")]
pub(crate) fn resident_bytes(pid: u32) -> Option<u64> {
    parse_linux_status(&std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?)
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_status(status: &str) -> Option<u64> {
    let value = status.lines().find_map(|line| line.strip_prefix("VmRSS:"))?;
    let mut parts = value.split_whitespace();
    let kib = parts.next()?.parse::<u64>().ok()?;
    if parts.next()? != "kB" || parts.next().is_some() {
        return None;
    }
    kib.checked_mul(1024)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn resident_bytes(_pid: u32) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_resident_memory_without_using_virtual_or_peak_size() {
        assert_eq!(parse_linux_status("VmSize:\t9000 kB\nVmRSS:\t1234 kB\nVmHWM:\t2000 kB\n"), Some(1234 * 1024));
        assert_eq!(parse_linux_status("VmRSS: 1234 MB"), None);
        assert_eq!(parse_linux_status("VmSize: 1234 kB"), None);
    }
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn measures_current_process_resident_memory() {
        assert!(resident_bytes(std::process::id()).is_some_and(|bytes| bytes > 0));
    }
}
