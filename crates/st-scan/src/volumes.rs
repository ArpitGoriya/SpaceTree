//! Enumerating the machine's drives, for the launcher's volume list.
//!
//! This lives in `st-scan` rather than `st-core` for the same reason the
//! MFT engine does: `st-core` stays portable and free of Win32, and this
//! is `GetLogicalDrives` + `GetVolumeInformationW` + `GetDiskFreeSpaceExW`.
//! On every other platform it returns nothing and the caller falls back
//! to its own enumeration (`/proc/mounts` on Linux) or the folder picker.
//!
//! Drives that report no media (an empty card reader, an open optical
//! drive) and network drives are left out: the first would show as a
//! 0-byte row, and the second is not something an MFT scan or a
//! full-drive walk should be started against by accident.

use st_core::volume::VolumeInfo;

/// One mounted volume, ready for the launcher to list.
pub struct VolumeEntry {
    /// The path a scan should be started against, e.g. `C:\`.
    pub path: String,
    pub info: VolumeInfo,
}

#[cfg(windows)]
pub fn list() -> Vec<VolumeEntry> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetDriveTypeW, GetLogicalDrives,
        GetVolumeInformationW,
    };
    use windows_sys::Win32::System::WindowsProgramming::{DRIVE_CDROM, DRIVE_REMOTE};

    // SAFETY: no arguments, no pointers — returns a bitmask of A..Z.
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();

    for bit in 0..26u32 {
        if mask & (1 << bit) == 0 {
            continue;
        }
        let letter = (b'A' + bit as u8) as char;
        let root = format!("{letter}:\\");
        let wide = wide_nul(&root);

        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
        if drive_type == DRIVE_REMOTE || drive_type == DRIVE_CDROM {
            continue;
        }

        let mut label_buf = [0u16; 256];
        let mut fs_buf = [0u16; 32];
        // SAFETY: both output buffers are passed with their own lengths;
        // the unused out-params are explicitly null.
        let ok = unsafe {
            GetVolumeInformationW(
                wide.as_ptr(),
                label_buf.as_mut_ptr(),
                label_buf.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs_buf.as_mut_ptr(),
                fs_buf.len() as u32,
            )
        };
        // Failure here means the drive letter exists but has no readable
        // volume behind it right now — an empty reader, or one we lack
        // rights to query. Either way there is nothing to list.
        if ok == 0 {
            continue;
        }

        let mut free_to_caller = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        // SAFETY: three valid out-pointers to stack locals.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_to_caller,
                &mut total,
                &mut total_free,
            )
        };
        if ok == 0 || total == 0 {
            continue;
        }

        // Cluster size is informational (the MFT engine reads it from
        // the boot sector itself), so a failure here is not worth
        // dropping the volume over.
        let mut sectors_per_cluster = 0u32;
        let mut bytes_per_sector = 0u32;
        // SAFETY: four valid out-pointers; the last two are unused but
        // must still be provided.
        let cluster_ok = unsafe {
            GetDiskFreeSpaceW(
                wide.as_ptr(),
                &mut sectors_per_cluster,
                &mut bytes_per_sector,
                &mut 0u32,
                &mut 0u32,
            )
        };
        let cluster_bytes = if cluster_ok == 0 {
            0
        } else {
            sectors_per_cluster.saturating_mul(bytes_per_sector)
        };

        let volume_label = from_wide(&label_buf);
        let filesystem = from_wide(&fs_buf);
        // Windows shows "Local Disk (C:)" for an unlabelled volume; the
        // letter is the part that identifies it, so it always appears.
        let label = if volume_label.is_empty() {
            format!("{letter}:")
        } else {
            format!("{volume_label} ({letter}:)")
        };

        out.push(VolumeEntry {
            path: root,
            info: VolumeInfo {
                label,
                filesystem,
                total_bytes: total,
                // "Free" is what this user can actually write, which is
                // what a quota-limited volume reports here, and matches
                // what Explorer shows.
                free_bytes: free_to_caller,
                cluster_bytes,
            },
        });
    }

    out
}

#[cfg(not(windows))]
pub fn list() -> Vec<VolumeEntry> {
    Vec::new()
}

#[cfg(windows)]
fn wide_nul(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
