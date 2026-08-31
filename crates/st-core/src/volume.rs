//! Drive/volume capacity — total, used and free bytes, plus filesystem
//! name and cluster size where the platform exposes it.
//!
//! Values come from the OS's own volume-info call (`GetDiskFreeSpaceExW`
//! on Windows, `statvfs` here), never by summing the scanned tree: a scan
//! root is often a subset of the volume, and even a full-volume scan
//! legitimately undercounts against "used" (`$MFT`, pagefile, System
//! Volume Information — see the size-semantics note in the plan).
//! Reporting both numbers and letting the UI show the delta is the
//! honest choice.
//!
//! The real Windows backend (`GetDiskFreeSpaceExW` /
//! `GetVolumeInformationW`) lives in `st-scan`, which is the only crate
//! allowed to touch the Win32 API — `st-core` stays fully portable and
//! testable on any host. The `statvfs` path below is what backs `query`
//! on Linux/macOS: real for local dev, CI and a future macOS engine, and
//! what the unit tests below exercise.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeInfo {
    pub label: String,
    pub filesystem: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub cluster_bytes: u32,
}

impl VolumeInfo {
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.free_bytes)
    }
}

// statvfs's block-count fields are `u64` on some unix targets and
// narrower on others (their libc type is platform-defined), so the `as
// u64` below is a real widening cast on those targets even though it's
// a same-type no-op on this one — hence the blanket allow rather than
// picking a form that only satisfies clippy on one platform.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
pub fn query(path: &std::path::Path) -> std::io::Result<VolumeInfo> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let block = stat.f_frsize as u64;
    Ok(VolumeInfo {
        label: path.display().to_string(),
        filesystem: "unknown".into(),
        total_bytes: stat.f_blocks as u64 * block,
        free_bytes: stat.f_bavail as u64 * block,
        cluster_bytes: stat.f_frsize as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn used_is_total_minus_free() {
        let v = VolumeInfo {
            label: "C:".into(),
            filesystem: "NTFS".into(),
            total_bytes: 1000,
            free_bytes: 300,
            cluster_bytes: 4096,
        };
        assert_eq!(v.used_bytes(), 700);
    }

    #[test]
    fn used_saturates_rather_than_underflows_on_bad_input() {
        let v = VolumeInfo {
            label: "C:".into(),
            filesystem: "NTFS".into(),
            total_bytes: 100,
            free_bytes: 300,
            cluster_bytes: 4096,
        };
        assert_eq!(v.used_bytes(), 0);
    }

    #[test]
    fn query_root_succeeds() {
        let info = query(std::path::Path::new("/")).expect("statvfs on / should succeed");
        assert!(info.total_bytes > 0);
    }
}
