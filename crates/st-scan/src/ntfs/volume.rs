//! Raw volume reads. The only Windows-specific I/O in the NTFS engine.
//!
//! Opening `\\.\C:` and reading it as a block device is what makes the
//! MFT approach possible at all — and it's also why this path needs
//! administrator rights, since it bypasses the filesystem entirely.

use std::io;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::GENERIC_READ;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, SetFilePointerEx, FILE_BEGIN, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

/// An open handle to a volume's raw bytes.
pub struct VolumeReader {
    handle: HANDLE,
    /// Reads through a raw volume handle must be aligned to this, in both
    /// offset and length — the kernel rejects anything else outright.
    sector_size: u32,
}

impl VolumeReader {
    /// Open `\\.\X:` for reading. Fails with a permission error when the
    /// process isn't elevated, which the caller treats as "fall back to
    /// the directory walker" rather than an error worth surfacing.
    pub fn open(drive_letter: char) -> io::Result<Self> {
        let path = format!(r"\\.\{}:", drive_letter.to_ascii_uppercase());
        let wide: Vec<u16> = std::ffi::OsStr::new(&path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives
        // the call; every other argument is a constant.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        // Start with the smallest sector size any volume uses; `open`'s
        // caller re-states the real one from the boot sector once it has
        // parsed it, and 512 divides every larger value so the bootstrap
        // read stays legal either way.
        Ok(Self {
            handle,
            sector_size: 512,
        })
    }

    /// Tell the reader the volume's true sector size, once the boot
    /// sector has been parsed.
    pub fn set_sector_size(&mut self, sector_size: u32) {
        if sector_size > 0 {
            self.sector_size = sector_size;
        }
    }

    pub fn sector_size(&self) -> u32 {
        self.sector_size
    }

    /// Read exactly `buf.len()` bytes starting at `offset`.
    ///
    /// Both must be sector-aligned. Callers read whole clusters, which
    /// are always a multiple of the sector size, so this holds naturally.
    pub fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let sector = self.sector_size as u64;
        if !offset.is_multiple_of(sector) || !(buf.len() as u64).is_multiple_of(sector) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "raw volume reads must be sector-aligned in both offset and length",
            ));
        }

        let mut position = 0usize;
        while position < buf.len() {
            // SAFETY: seeking a valid handle to a non-negative offset.
            let ok = unsafe {
                SetFilePointerEx(
                    self.handle,
                    (offset + position as u64) as i64,
                    std::ptr::null_mut(),
                    FILE_BEGIN,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }

            // A single ReadFile can return short; loop until satisfied.
            let remaining = &mut buf[position..];
            let mut read: u32 = 0;
            // SAFETY: `remaining` is a valid writable slice of the stated
            // length, and `read` outlives the call.
            let ok = unsafe {
                ReadFile(
                    self.handle,
                    remaining.as_mut_ptr(),
                    remaining.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "volume read returned no data before the requested length was filled",
                ));
            }
            position += read as usize;
        }
        Ok(())
    }
}

impl Drop for VolumeReader {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            // SAFETY: the handle came from CreateFileW and is closed once.
            unsafe { CloseHandle(self.handle) };
        }
    }
}

// The handle is only ever used through `&self` methods that don't mutate
// shared state, and Windows file handles are safe to use from any thread.
unsafe impl Send for VolumeReader {}
unsafe impl Sync for VolumeReader {}
