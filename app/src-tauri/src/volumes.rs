//! Local volume enumeration for the launcher screen.
//!
//! Two implementations, one per platform this app actually runs on:
//! Windows drives come from `st_scan::volumes` (`GetLogicalDrives` and
//! friends — Win32 stays confined to `st-scan`), and Linux mounts are
//! read from `/proc/mounts` with virtual/pseudo filesystems filtered
//! out, which is what makes the launcher testable in this project's own
//! dev/CI environment. Anywhere else the list is empty and the "Scan a
//! folder…" picker — a native dialog, genuinely cross-platform — is the
//! way in.

#[cfg(target_os = "linux")]
use st_core::volume;

use crate::dto::VolumeDto;

#[cfg(target_os = "linux")]
const SKIP_FSTYPES: &[&str] = &[
    "proc",
    "sysfs",
    "cgroup",
    "cgroup2",
    "devpts",
    "tmpfs",
    "devtmpfs",
    "mqueue",
    "debugfs",
    "tracefs",
    "securityfs",
    "pstore",
    "bpf",
    "overlay",
    "squashfs",
    "autofs",
    "hugetlbfs",
    "binfmt_misc",
    "configfs",
    "fusectl",
    "ramfs",
    "nsfs",
    "rpc_pipefs",
];

#[cfg(target_os = "linux")]
pub fn list_volumes() -> Vec<VolumeDto> {
    let Ok(content) = std::fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for line in content.lines() {
        let mut fields = line.split_whitespace();
        let Some(device) = fields.next() else {
            continue;
        };
        let Some(mount_point) = fields.next() else {
            continue;
        };
        let Some(fstype) = fields.next() else {
            continue;
        };
        if SKIP_FSTYPES.contains(&fstype) {
            continue;
        }
        // /proc/mounts escapes spaces etc. as octal; not worth unescaping
        // for a label users only ever read, so just leave it as-is.
        let Ok(info) = volume::query(std::path::Path::new(mount_point)) else {
            continue;
        };
        if info.total_bytes == 0 {
            continue;
        }
        let label = if mount_point == "/" {
            device.to_string()
        } else {
            mount_point.to_string()
        };
        let mut dto = VolumeDto::new(mount_point.to_string(), label, &info);
        dto.filesystem = fstype.to_string();
        out.push(dto);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

#[cfg(windows)]
pub fn list_volumes() -> Vec<VolumeDto> {
    st_scan::volumes::list()
        .into_iter()
        .map(|entry| VolumeDto::new(entry.path, entry.info.label.clone(), &entry.info))
        .collect()
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn list_volumes() -> Vec<VolumeDto> {
    Vec::new()
}
