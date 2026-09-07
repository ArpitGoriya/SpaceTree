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

/// What one parsed record contributes to the live progress counters:
/// one file and its on-disk bytes, or nothing.
///
/// This is deliberately a free function in the portable half of the
/// module rather than a branch inside the Windows-only engine, because
/// it encodes the rule that got this wrong once and it needs a test.
/// The engine's read loop knows only how many bytes of the Master File
/// Table it has consumed — a quantity that converges on the size of
/// `$MFT` (two or three GB) regardless of how full the volume is. Byte
/// totals have to come from the records' own `$DATA` sizes, and only
/// from records that represent a real file:
///
/// - Directories carry no bytes of their own (their subtree totals come
///   from the rollup), matching the walker.
/// - Extension records hold the overflow attributes of another record;
///   their bytes belong to the base record and would double-count here.
/// - Records not in use are deleted files that no longer occupy space.
pub fn progress_contribution(rec: &record::FileRecord) -> Option<u64> {
    if !rec.in_use || rec.base_record != 0 || rec.is_dir {
        return None;
    }
    Some(rec.size_alloc)
}

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

    fn rec(size_alloc: u64) -> record::FileRecord {
        record::FileRecord {
            record_number: Some(42),
            in_use: true,
            is_dir: false,
            base_record: 0,
            hard_link_count: 1,
            name: Some("a.bin".into()),
            parent: Some(5),
            mtime: 0,
            size_logical: size_alloc,
            size_alloc,
            has_attribute_list: false,
        }
    }

    /// The regression this function exists for: progress must be driven
    /// by the files' own sizes, never by how much of the MFT has been
    /// read. The read-loop version stalled at ~2 GB (the size of `$MFT`)
    /// on a 234 GB drive.
    #[test]
    fn progress_counts_file_bytes_only() {
        assert_eq!(progress_contribution(&rec(4096)), Some(4096));

        let mut dir = rec(4096);
        dir.is_dir = true;
        assert_eq!(
            progress_contribution(&dir),
            None,
            "a directory carries no bytes of its own; the rollup supplies its subtree total"
        );

        let mut deleted = rec(4096);
        deleted.in_use = false;
        assert_eq!(
            progress_contribution(&deleted),
            None,
            "a deleted record no longer occupies space"
        );

        let mut extension = rec(4096);
        extension.base_record = 7;
        assert_eq!(
            progress_contribution(&extension),
            None,
            "an extension record's bytes belong to its base record and would double-count"
        );

        assert_eq!(
            progress_contribution(&rec(0)),
            Some(0),
            "an empty file is still a file"
        );
    }

    /// A whole volume's worth of records must total the files' bytes, not
    /// anything proportional to the number of records.
    #[test]
    fn a_volume_of_records_totals_the_file_bytes() {
        let mut records = Vec::new();
        for _ in 0..1000 {
            records.push(rec(1_000_000));
        }
        // Plenty of directories and deleted records mixed in, as on any
        // real volume.
        for _ in 0..4000 {
            let mut d = rec(999);
            d.is_dir = true;
            records.push(d);
        }
        for _ in 0..2000 {
            let mut d = rec(999);
            d.in_use = false;
            records.push(d);
        }

        let total: u64 = records.iter().filter_map(progress_contribution).sum();
        let files = records.iter().filter_map(progress_contribution).count();
        assert_eq!(total, 1_000_000_000);
        assert_eq!(files, 1000);
    }

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
