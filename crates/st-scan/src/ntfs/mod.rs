//! NTFS Master File Table reader — "Engine A" from docs/PLAN.md.
//!
//! This is what makes a whole-drive scan finish in seconds instead of
//! minutes: rather than walking directories (millions of syscalls), it
//! reads the MFT — one flat on-disk table with a record per file — in
//! large sequential reads, then rebuilds the hierarchy in memory from
//! each record's parent reference.
//!
//! **Layering, and why it's arranged this way:** everything that decodes
//! bytes ([`boot`], [`runlist`], [`record`]) is plain
//! `&[u8] -> Result<T>` code that compiles and is unit-tested on every
//! platform, against handcrafted structures. Only [`volume`] (raw device
//! I/O) and [`elevation`] are Windows-gated. That split is deliberate:
//! the hard, easy-to-get-wrong part — fixups, attribute walking, runlist
//! decoding — is the part that can be tested anywhere, so it isn't
//! taken on trust just because it happens to compile for Windows.
//!
//! **Contract:** this parses on-disk structures that a corrupt or
//! hostile volume controls, so nothing here panics, loops forever, or
//! indexes out of bounds. Every read is bounds-checked and every
//! arithmetic op that could overflow is checked; a bad record yields
//! `Err` and is skipped, and a bad enough volume makes the whole engine
//! bail so the caller can fall back to the directory walker.

pub mod boot;
pub mod record;
pub mod runlist;

#[cfg(windows)]
pub mod elevation;
#[cfg(windows)]
pub mod volume;

#[cfg(windows)]
mod engine;
#[cfg(windows)]
pub use engine::scan_volume;

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NtfsError {
    /// A read ran past the end of the buffer it was given.
    OutOfBounds {
        offset: usize,
        len: usize,
        buf_len: usize,
    },
    /// Structure signature didn't match (not NTFS, not a FILE record…).
    BadMagic,
    /// A field held a value that can't describe a real volume — a
    /// zero sector size, a record smaller than its own header, and so on.
    BadField(&'static str),
    /// Fixup application failed: the update sequence number in a sector
    /// footer didn't match the record's, meaning the record is torn.
    FixupMismatch,
}

impl fmt::Display for NtfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NtfsError::OutOfBounds {
                offset,
                len,
                buf_len,
            } => {
                write!(
                    f,
                    "read of {len} bytes at offset {offset} exceeds buffer of {buf_len}"
                )
            }
            NtfsError::BadMagic => write!(f, "structure signature did not match"),
            NtfsError::BadField(what) => write!(f, "implausible value for {what}"),
            NtfsError::FixupMismatch => write!(f, "update sequence number mismatch (torn record)"),
        }
    }
}

impl std::error::Error for NtfsError {}

pub type Result<T> = std::result::Result<T, NtfsError>;

/// Bounds-checked little-endian reads. Every field access in this module
/// goes through these rather than slicing directly, so a truncated or
/// malformed buffer produces an error instead of a panic.
pub(crate) fn u8_at(buf: &[u8], offset: usize) -> Result<u8> {
    buf.get(offset).copied().ok_or(NtfsError::OutOfBounds {
        offset,
        len: 1,
        buf_len: buf.len(),
    })
}

pub(crate) fn u16_at(buf: &[u8], offset: usize) -> Result<u16> {
    let b = slice_at(buf, offset, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

pub(crate) fn u32_at(buf: &[u8], offset: usize) -> Result<u32> {
    let b = slice_at(buf, offset, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn u64_at(buf: &[u8], offset: usize) -> Result<u64> {
    let b = slice_at(buf, offset, 8)?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

pub(crate) fn slice_at(buf: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    let end = offset.checked_add(len).ok_or(NtfsError::OutOfBounds {
        offset,
        len,
        buf_len: buf.len(),
    })?;
    buf.get(offset..end).ok_or(NtfsError::OutOfBounds {
        offset,
        len,
        buf_len: buf.len(),
    })
}

/// Windows `FILETIME` (100ns ticks since 1601-01-01) to Unix seconds.
/// Timestamps before the Unix epoch — including the zeroes that unused
/// records are full of — clamp to 0 rather than going negative, since
/// the UI renders a 0 as "no date" and a negative as a 1601 date.
pub(crate) fn filetime_to_unix_secs(filetime: u64) -> i64 {
    const TICKS_PER_SEC: u64 = 10_000_000;
    const EPOCH_DIFF_SECS: u64 = 11_644_473_600;
    let secs = filetime / TICKS_PER_SEC;
    secs.saturating_sub(EPOCH_DIFF_SECS) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_checked_reads_error_instead_of_panicking() {
        let buf = [1u8, 2, 3];
        assert!(u32_at(&buf, 0).is_err());
        assert!(u16_at(&buf, 2).is_err());
        assert_eq!(u16_at(&buf, 0).unwrap(), 0x0201);
        assert!(
            slice_at(&buf, usize::MAX, 8).is_err(),
            "offset+len overflow must not wrap"
        );
    }

    #[test]
    fn filetime_converts_and_clamps() {
        // 1970-01-01 exactly.
        assert_eq!(filetime_to_unix_secs(11_644_473_600 * 10_000_000), 0);
        // One hour later.
        assert_eq!(
            filetime_to_unix_secs((11_644_473_600 + 3600) * 10_000_000),
            3600
        );
        // Unset timestamps in unused records must not become 1601.
        assert_eq!(filetime_to_unix_secs(0), 0);
    }
}
