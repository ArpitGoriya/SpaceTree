//! Scan engines that populate an `st_core::TreeBuilder`.
//!
//! Two engines, picked automatically by [`scan_auto`]:
//!
//! - **NTFS MFT** ([`ntfs`], Windows only) — reads the volume's Master
//!   File Table directly instead of walking directories. Needs
//!   administrator rights and an NTFS volume, and is an order of
//!   magnitude faster because it turns millions of directory syscalls
//!   into a handful of large sequential reads.
//! - **Directory walker** ([`walker`]) — portable, needs no privileges,
//!   and works on any filesystem and on individual folders. Used
//!   whenever the MFT path doesn't apply or fails.

mod dirlist;
pub mod ntfs;
pub mod volumes;
pub mod walker;

pub use walker::{scan, ScanPhase, ScanProgress, ScanResult};

use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Scan `path`, using the fastest engine available for it.
///
/// Falls back to the directory walker whenever the MFT engine isn't
/// applicable (not Windows, not a whole NTFS volume, not elevated) or
/// fails partway — a slower scan is a far better outcome than an error,
/// and the caller can tell which ran from [`ScanResult::engine`].
// `on_progress` is borrowed mutably by the MFT attempt below, which only
// exists on Windows; everywhere else it's moved straight into the walker
// and the `mut` reads as redundant.
#[cfg_attr(not(windows), allow(unused_mut))]
pub fn scan_auto(
    path: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(ScanProgress) + Send,
) -> std::io::Result<ScanResult> {
    #[cfg(windows)]
    {
        if let Some(letter) = whole_ntfs_volume(path) {
            if ntfs::elevation::is_elevated() {
                match ntfs::scan_volume(letter, cancel, &mut on_progress) {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        log_fallback(&e);
                    }
                }
            }
        }
    }
    walker::scan(path, cancel, on_progress)
}

/// Whether the MFT engine could be used for `path`, ignoring elevation.
/// Lets the UI offer a "fast scan needs administrator" affordance instead
/// of silently running the slow path.
#[cfg(windows)]
pub fn can_use_fast_engine(path: &Path) -> bool {
    whole_ntfs_volume(path).is_some()
}

#[cfg(not(windows))]
pub fn can_use_fast_engine(_path: &Path) -> bool {
    false
}

/// Returns the drive letter when `path` is the root of an NTFS volume.
///
/// The MFT covers a whole volume, so it only answers questions about a
/// whole volume; scanning one folder still goes through the walker.
#[cfg(windows)]
fn whole_ntfs_volume(path: &Path) -> Option<char> {
    let text = path.to_str()?;
    let bytes = text.as_bytes();
    // "C:", "C:\" or "C:/" — anything deeper is a folder scan.
    let is_root = matches!(bytes.len(), 2 | 3)
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes.get(2).is_none_or(|c| *c == b'\\' || *c == b'/');
    if !is_root {
        return None;
    }
    let letter = bytes[0] as char;
    is_ntfs(letter).then_some(letter)
}

#[cfg(windows)]
fn is_ntfs(drive_letter: char) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    let root = format!("{}:\\", drive_letter.to_ascii_uppercase());
    let wide: Vec<u16> = std::ffi::OsStr::new(&root)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut fs_name = [0u16; 32];

    // SAFETY: `wide` is NUL-terminated and outlives the call; the output
    // buffer length matches the array.
    let ok = unsafe {
        GetVolumeInformationW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    if ok == 0 {
        return false;
    }
    let end = fs_name
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(fs_name.len());
    String::from_utf16_lossy(&fs_name[..end]).eq_ignore_ascii_case("NTFS")
}

#[cfg(windows)]
fn log_fallback(error: &std::io::Error) {
    // The MFT path is best-effort: a volume it can't read (BitLocker,
    // an unusual layout, corruption) is a reason to walk directories,
    // not to fail the scan.
    eprintln!("spacetree: MFT engine unavailable, falling back to walker: {error}");
}
