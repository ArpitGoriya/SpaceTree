//! Reading one directory's entries, as fast as each platform allows.
//!
//! The walker's inner loop is "list a directory, get each entry's type,
//! size and timestamp", and on Windows the obvious implementation is
//! quietly expensive. `std::fs::read_dir` calls `FindFirstFileW`, which
//! looks up every entry's 8.3 short name, and `DirEntry::metadata()`
//! opens a handle to any reparse point to read its tag — so a drive with
//! OneDrive placeholders or junctions pays a `CreateFileW` per entry.
//!
//! This module gets all of it from a single `FindFirstFileExW` scan:
//! `FindExInfoBasic` skips the short-name lookup, `FIND_FIRST_EX_LARGE_FETCH`
//! batches the kernel round-trips, and the reparse tag is read straight
//! out of the result the API already filled in.

use std::io;
use std::path::Path;

/// One directory entry, with everything the walker needs already read.
pub struct RawDirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size_logical: u64,
    pub size_alloc: u64,
    pub mtime: i64,
    /// `(device, inode)` for a file with more than one link, so the
    /// walker can count its bytes once. Taken from metadata the listing
    /// already read, never a second stat. Unix only — Windows hard-link
    /// identity needs an extra open per file, which is exactly the cost
    /// this module exists to avoid; the MFT engine handles it there.
    /// The field stays in the struct on every platform so both `list`
    /// implementations keep one shape; only Unix ever reads it.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub hardlink_key: Option<(u64, u64)>,
}

#[cfg(windows)]
pub fn list(dir: &Path) -> io::Result<Vec<RawDirEntry>> {
    windows_impl::list(dir)
}

#[cfg(not(windows))]
pub fn list(dir: &Path) -> io::Result<Vec<RawDirEntry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        // Does not follow symlinks, matching the "never traverse a
        // reparse point" policy.
        let Ok(meta) = entry.metadata() else { continue };
        let is_symlink = meta.is_symlink();
        let is_dir = meta.is_dir();
        out.push(RawDirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir,
            is_symlink,
            size_logical: if is_dir { 0 } else { meta.len() },
            size_alloc: if is_dir { 0 } else { alloc_size(&meta) },
            mtime: mtime_secs(&meta),
            hardlink_key: hardlink_key(&meta),
        });
    }
    Ok(out)
}

#[cfg(unix)]
fn alloc_size(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

#[cfg(not(any(unix, windows)))]
fn alloc_size(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

#[cfg(not(windows))]
fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    use std::time::UNIX_EPOCH;
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn hardlink_key(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    (meta.nlink() > 1).then(|| (meta.dev(), meta.ino()))
}

#[cfg(not(any(unix, windows)))]
fn hardlink_key(_meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

#[cfg(windows)]
mod windows_impl {
    use super::RawDirEntry;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH,
        WIN32_FIND_DATAW,
    };

    pub fn list(dir: &Path) -> io::Result<Vec<RawDirEntry>> {
        let mut pattern = extended_path(dir);
        // `FindFirstFileEx` wants a search pattern, not a directory.
        if pattern.last() != Some(&(b'\\' as u16)) {
            pattern.push(b'\\' as u16);
        }
        pattern.push(b'*' as u16);
        pattern.push(0);

        // SAFETY: zeroed is a valid initial state for this POD struct;
        // the API fills it before we read any field.
        let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
        // SAFETY: `pattern` is a NUL-terminated UTF-16 string outliving
        // the call, and `data` is a valid writable out-param.
        let handle = unsafe {
            FindFirstFileExW(
                pattern.as_ptr(),
                FindExInfoBasic,
                (&raw mut data).cast(),
                FindExSearchNameMatch,
                std::ptr::null(),
                FIND_FIRST_EX_LARGE_FETCH,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut out = Vec::new();
        loop {
            if let Some(entry) = convert(&data) {
                out.push(entry);
            }
            // SAFETY: `handle` is live until FindClose below.
            if unsafe { FindNextFileW(handle, &mut data) } == 0 {
                let err = io::Error::last_os_error();
                // SAFETY: closing a handle obtained above, exactly once.
                unsafe { FindClose(handle) };
                return if err.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                    Ok(out)
                } else {
                    Err(err)
                };
            }
        }
    }

    fn convert(data: &WIN32_FIND_DATAW) -> Option<RawDirEntry> {
        let name = file_name(data)?;
        if name == "." || name == ".." {
            return None;
        }

        let attrs = data.dwFileAttributes;
        let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
        // The reparse tag is already in the result — reading it here is
        // what saves a `CreateFileW` per link compared with std.
        let is_symlink = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;

        let size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
        let size_logical = if is_dir || is_symlink { 0 } else { size };

        Some(RawDirEntry {
            name,
            is_dir,
            is_symlink,
            size_logical,
            // `WIN32_FIND_DATAW` carries no allocated size, and asking for
            // one would mean an extra open per file — exactly the cost
            // this path exists to avoid. The MFT engine reports true
            // on-disk sizes; here the two columns coincide.
            size_alloc: size_logical,
            mtime: filetime_to_unix(
                ((data.ftLastWriteTime.dwHighDateTime as u64) << 32)
                    | data.ftLastWriteTime.dwLowDateTime as u64,
            ),
            hardlink_key: None,
        })
    }

    fn file_name(data: &WIN32_FIND_DATAW) -> Option<String> {
        let end = data
            .cFileName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(data.cFileName.len());
        if end == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&data.cFileName[..end]))
    }

    fn filetime_to_unix(filetime: u64) -> i64 {
        crate::ntfs::filetime_to_unix_secs(filetime)
    }

    /// Prefix an absolute path with `\\?\` so it isn't subject to the
    /// 260-character limit. Paths that are already extended, or that
    /// aren't plain absolute drive paths, are left alone rather than
    /// risking a malformed prefix.
    fn extended_path(path: &Path) -> Vec<u16> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        let already_extended =
            wide.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]);
        let drive_absolute = wide.len() >= 3
            && (wide[0] as u8 as char).is_ascii_alphabetic()
            && wide[1] == b':' as u16
            && wide[2] == b'\\' as u16;

        if already_extended || !drive_absolute {
            return wide;
        }
        let mut prefixed: Vec<u16> = r"\\?\".encode_utf16().collect();
        prefixed.extend_from_slice(&wide);
        prefixed
    }
}
