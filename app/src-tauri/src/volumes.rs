//! Local volume enumeration for the launcher screen.
//!
//! Implemented today for Linux only, by reading `/proc/mounts` and
//! filtering out virtual/pseudo filesystems — real, working, and
//! directly testable in this project's Linux dev/CI environment. The
//! plan's actual target (`GetLogicalDrives` / `GetVolumeInformationW` on
//! Windows) needs a Windows machine to write and verify, so rather than
//! guess at that API surface, non-Linux platforms return an empty list
//! for now; the "Scan a folder…" picker (a native dialog, genuinely
//! cross-platform) is what the launcher falls back to until then.

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

#[cfg(not(target_os = "linux"))]
pub fn list_volumes() -> Vec<VolumeDto> {
    Vec::new()
}
