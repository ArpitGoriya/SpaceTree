//! MFT FILE record parsing: fixup repair, attribute walking, and
//! extraction of the name, parent, timestamps and sizes that the scan
//! tree needs.

use super::{filetime_to_unix_secs, slice_at, u16_at, u32_at, u64_at, u8_at, NtfsError, Result};

const MAGIC_FILE: &[u8] = b"FILE";

const FLAG_IN_USE: u16 = 0x0001;
const FLAG_DIRECTORY: u16 = 0x0002;

const ATTR_STANDARD_INFORMATION: u32 = 0x10;
const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_END: u32 = 0xFFFF_FFFF;

/// Smallest attribute record that can exist (the common header). Used as
/// the loop's minimum stride so a zero or nonsense length can't spin.
const MIN_ATTR_LEN: usize = 16;

/// `$FILE_NAME` namespaces. A file with an 8.3 alias carries a second
/// `$FILE_NAME` in the DOS namespace holding a mangled name like
/// `PROGRA~1`; taking it would put the wrong text in the tree.
const NAMESPACE_POSIX: u8 = 0;
const NAMESPACE_WIN32: u8 = 1;
const NAMESPACE_DOS: u8 = 2;
const NAMESPACE_WIN32_DOS: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileRecord {
    /// Self-reported record number (NTFS 3.0+). The reader trusts its
    /// own position instead, but a mismatch is a useful sanity signal.
    pub record_number: Option<u64>,
    pub in_use: bool,
    pub is_dir: bool,
    /// 0 for a base record; otherwise the record this one extends.
    pub base_record: u64,
    pub hard_link_count: u16,
    pub name: Option<String>,
    /// MFT record number of the containing directory.
    pub parent: Option<u64>,
    pub mtime: i64,
    pub size_logical: u64,
    pub size_alloc: u64,
    /// The record's attributes spill into other records. Sizes here may
    /// be incomplete; the caller can decide whether to chase them.
    pub has_attribute_list: bool,
}

/// Repair a record in place by undoing the update sequence array.
///
/// NTFS steals the last two bytes of every sector in a record and writes
/// an incrementing number there, keeping the real bytes in an array in
/// the header. That lets it detect a torn write — if one sector of a
/// multi-sector record didn't make it to disk, its footer won't match.
/// Nothing in the record can be read until this is undone, since
/// attributes straddle sector boundaries.
pub fn apply_fixups(buf: &mut [u8], bytes_per_sector: u32) -> Result<()> {
    if slice_at(buf, 0, 4)? != MAGIC_FILE {
        return Err(NtfsError::BadMagic);
    }
    let usa_offset = u16_at(buf, 0x04)? as usize;
    let usa_count = u16_at(buf, 0x06)? as usize;
    if usa_count == 0 {
        return Err(NtfsError::BadField("update sequence array count"));
    }

    let sectors = usa_count - 1;
    let bytes_per_sector = bytes_per_sector as usize;
    if bytes_per_sector < 4 {
        return Err(NtfsError::BadField("bytes per sector"));
    }
    let covered = sectors
        .checked_mul(bytes_per_sector)
        .ok_or(NtfsError::BadField("update sequence array span"))?;
    if covered > buf.len() {
        return Err(NtfsError::BadField("update sequence array exceeds record"));
    }

    let usn = u16_at(buf, usa_offset)?;
    for sector in 0..sectors {
        let replacement = u16_at(buf, usa_offset + 2 + sector * 2)?;
        let footer = (sector + 1) * bytes_per_sector - 2;
        if u16_at(buf, footer)? != usn {
            return Err(NtfsError::FixupMismatch);
        }
        let bytes = replacement.to_le_bytes();
        buf[footer] = bytes[0];
        buf[footer + 1] = bytes[1];
    }
    Ok(())
}

/// Parse a fixed-up FILE record.
///
/// Call [`apply_fixups`] first — parsing raw bytes straight off the disk
/// yields corrupt values wherever an attribute crosses a sector boundary.
pub fn parse(buf: &[u8]) -> Result<FileRecord> {
    if slice_at(buf, 0, 4)? != MAGIC_FILE {
        return Err(NtfsError::BadMagic);
    }

    let flags = u16_at(buf, 0x16)?;
    let mut record = FileRecord {
        record_number: u32_at(buf, 0x2C).ok().map(u64::from),
        in_use: flags & FLAG_IN_USE != 0,
        is_dir: flags & FLAG_DIRECTORY != 0,
        base_record: u64_at(buf, 0x20)? & 0x0000_FFFF_FFFF_FFFF,
        hard_link_count: u16_at(buf, 0x12)?,
        ..Default::default()
    };

    // Unused records are leftover space in the table; their attribute
    // area is stale and not worth walking.
    if !record.in_use {
        return Ok(record);
    }

    let first_attr = u16_at(buf, 0x14)? as usize;
    let used_size = u32_at(buf, 0x18)? as usize;
    let limit = used_size.min(buf.len());

    let mut best_name_rank = 0u8;
    let mut saw_unnamed_data = false;
    let mut offset = first_attr;

    while offset + 8 <= limit {
        let attr_type = u32_at(buf, offset)?;
        if attr_type == ATTR_END {
            break;
        }
        let attr_len = u32_at(buf, offset + 4)? as usize;
        // A short or zero length would either overlap the previous
        // attribute or stall the walk; either way the record is junk.
        if attr_len < MIN_ATTR_LEN || offset + attr_len > limit {
            break;
        }
        let attr = slice_at(buf, offset, attr_len)?;

        match attr_type {
            ATTR_STANDARD_INFORMATION => {
                if let Ok(value) = resident_value(attr) {
                    if let Ok(ft) = u64_at(value, 0x08) {
                        record.mtime = filetime_to_unix_secs(ft);
                    }
                }
            }
            ATTR_ATTRIBUTE_LIST => record.has_attribute_list = true,
            ATTR_FILE_NAME => {
                if let Ok(value) = resident_value(attr) {
                    if let Ok((name, parent, rank)) = parse_file_name(value) {
                        if rank > best_name_rank {
                            best_name_rank = rank;
                            record.name = Some(name);
                            record.parent = Some(parent);
                        }
                    }
                }
            }
            ATTR_DATA => {
                let named = u8_at(attr, 0x09)? != 0;
                let (logical, alloc) = data_sizes(attr)?;
                if named {
                    // An alternate data stream: real bytes belonging to
                    // this file, so they count toward it, but it gets no
                    // row of its own in the tree.
                    record.size_logical = record.size_logical.saturating_add(logical);
                    record.size_alloc = record.size_alloc.saturating_add(alloc);
                } else if !saw_unnamed_data {
                    saw_unnamed_data = true;
                    record.size_logical = record.size_logical.saturating_add(logical);
                    record.size_alloc = record.size_alloc.saturating_add(alloc);
                }
            }
            _ => {}
        }

        offset += attr_len;
    }

    // Directories carry no size of their own; the tree's rollup supplies
    // their subtree totals, exactly as with the directory walker.
    if record.is_dir {
        record.size_logical = 0;
        record.size_alloc = 0;
    }

    Ok(record)
}

/// Returns the raw runlist bytes of the record's unnamed, non-resident
/// `$DATA` attribute.
///
/// This is the bootstrap step for reading the MFT at all: `$MFT`'s own
/// record describes where the table is physically laid out, so the
/// reader parses record 0, pulls this runlist, and follows it to stream
/// the rest.
pub fn unnamed_data_runlist(buf: &[u8]) -> Result<Option<&[u8]>> {
    if slice_at(buf, 0, 4)? != MAGIC_FILE {
        return Err(NtfsError::BadMagic);
    }
    let first_attr = u16_at(buf, 0x14)? as usize;
    let used_size = u32_at(buf, 0x18)? as usize;
    let limit = used_size.min(buf.len());

    let mut offset = first_attr;
    while offset + 8 <= limit {
        let attr_type = u32_at(buf, offset)?;
        if attr_type == ATTR_END {
            break;
        }
        let attr_len = u32_at(buf, offset + 4)? as usize;
        if attr_len < MIN_ATTR_LEN || offset + attr_len > limit {
            break;
        }
        let attr = slice_at(buf, offset, attr_len)?;

        let non_resident = u8_at(attr, 0x08)? != 0;
        let named = u8_at(attr, 0x09)? != 0;
        if attr_type == ATTR_DATA && non_resident && !named {
            let runlist_offset = u16_at(attr, 0x20)? as usize;
            return Ok(Some(slice_at(
                attr,
                runlist_offset,
                attr_len - runlist_offset,
            )?));
        }
        offset += attr_len;
    }
    Ok(None)
}

/// The value bytes of a resident attribute. Non-resident attributes hold
/// a runlist rather than a value, so callers that need one must check.
fn resident_value(attr: &[u8]) -> Result<&[u8]> {
    if u8_at(attr, 0x08)? != 0 {
        return Err(NtfsError::BadField("attribute is non-resident"));
    }
    let value_len = u32_at(attr, 0x10)? as usize;
    let value_off = u16_at(attr, 0x14)? as usize;
    slice_at(attr, value_off, value_len)
}

/// Returns `(logical, allocated)` for a `$DATA` attribute.
///
/// A resident stream lives inside the MFT record itself and occupies no
/// clusters of its own, so its allocated size is 0 — which is what
/// Windows reports as "size on disk" for such files too. For a
/// non-resident stream the header's allocated size already accounts for
/// compression and sparseness.
fn data_sizes(attr: &[u8]) -> Result<(u64, u64)> {
    if u8_at(attr, 0x08)? == 0 {
        let value_len = u32_at(attr, 0x10)? as u64;
        return Ok((value_len, 0));
    }
    // Only the fragment starting at VCN 0 carries the stream's totals;
    // later fragments repeat the runlist but not meaningful sizes.
    if u64_at(attr, 0x10)? != 0 {
        return Ok((0, 0));
    }
    let allocated = u64_at(attr, 0x28)?;
    let real = u64_at(attr, 0x30)?;
    Ok((real, allocated))
}

/// Returns `(name, parent_record_number, namespace_rank)`.
fn parse_file_name(value: &[u8]) -> Result<(String, u64, u8)> {
    let parent = u64_at(value, 0x00)? & 0x0000_FFFF_FFFF_FFFF;
    let name_len = u8_at(value, 0x40)? as usize;
    let namespace = u8_at(value, 0x41)?;
    let bytes = slice_at(value, 0x42, name_len * 2)?;

    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let name = String::from_utf16_lossy(&units);

    // Higher wins. A DOS-only entry is the 8.3 alias of a name that also
    // appears in another namespace, so it ranks lowest — but is still
    // accepted, since it beats having no name at all.
    let rank = match namespace {
        NAMESPACE_WIN32 | NAMESPACE_WIN32_DOS => 3,
        NAMESPACE_POSIX => 2,
        NAMESPACE_DOS => 1,
        _ => 1,
    };
    Ok((name, parent, rank))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECTOR: u32 = 512;
    const RECORD: usize = 1024;

    /// Builds a FILE record the way NTFS lays one out, so the parser is
    /// exercised against real structure rather than a convenient mock.
    struct RecordBuilder {
        buf: Vec<u8>,
        attrs: Vec<u8>,
        flags: u16,
    }

    impl RecordBuilder {
        fn new() -> Self {
            Self {
                buf: vec![0u8; RECORD],
                attrs: Vec::new(),
                flags: FLAG_IN_USE,
            }
        }

        fn directory(mut self) -> Self {
            self.flags |= FLAG_DIRECTORY;
            self
        }

        fn not_in_use(mut self) -> Self {
            self.flags &= !FLAG_IN_USE;
            self
        }

        fn standard_information(mut self, mtime_filetime: u64) -> Self {
            let mut value = vec![0u8; 0x48];
            value[0x08..0x10].copy_from_slice(&mtime_filetime.to_le_bytes());
            self.attrs
                .extend(resident_attr(ATTR_STANDARD_INFORMATION, &value, false));
            self
        }

        fn file_name(mut self, name: &str, parent: u64, namespace: u8) -> Self {
            let units: Vec<u16> = name.encode_utf16().collect();
            let mut value = vec![0u8; 0x42 + units.len() * 2];
            value[0x00..0x08].copy_from_slice(&parent.to_le_bytes());
            value[0x40] = units.len() as u8;
            value[0x41] = namespace;
            for (i, u) in units.iter().enumerate() {
                value[0x42 + i * 2..0x44 + i * 2].copy_from_slice(&u.to_le_bytes());
            }
            self.attrs
                .extend(resident_attr(ATTR_FILE_NAME, &value, false));
            self
        }

        fn resident_data(mut self, contents: &[u8]) -> Self {
            self.attrs.extend(resident_attr(ATTR_DATA, contents, false));
            self
        }

        fn nonresident_data(mut self, real: u64, allocated: u64, start_vcn: u64) -> Self {
            self.attrs.extend(nonresident_attr(
                ATTR_DATA, real, allocated, start_vcn, false,
            ));
            self
        }

        fn data_with_runs(mut self, runs: &[u8]) -> Self {
            self.attrs
                .extend(nonresident_attr_with_runs(ATTR_DATA, 0, 0, 0, false, runs));
            self
        }

        fn named_stream(mut self, real: u64, allocated: u64) -> Self {
            self.attrs
                .extend(nonresident_attr(ATTR_DATA, real, allocated, 0, true));
            self
        }

        fn attribute_list(mut self) -> Self {
            self.attrs
                .extend(resident_attr(ATTR_ATTRIBUTE_LIST, &[0u8; 8], false));
            self
        }

        /// Lays the header and attributes out, then writes the update
        /// sequence array over each sector footer the way the filesystem
        /// does, so `apply_fixups` has something real to undo.
        fn build(mut self) -> Vec<u8> {
            let first_attr = 0x38usize;
            self.buf[0x00..0x04].copy_from_slice(MAGIC_FILE);
            self.buf[0x04..0x06].copy_from_slice(&0x30u16.to_le_bytes()); // usa offset
            self.buf[0x06..0x08].copy_from_slice(&3u16.to_le_bytes()); // usn + 2 sectors
            self.buf[0x12..0x14].copy_from_slice(&1u16.to_le_bytes()); // hard links
            self.buf[0x14..0x16].copy_from_slice(&(first_attr as u16).to_le_bytes());
            self.buf[0x16..0x18].copy_from_slice(&self.flags.to_le_bytes());
            self.buf[0x2C..0x30].copy_from_slice(&42u32.to_le_bytes()); // record number

            let end = first_attr + self.attrs.len();
            self.buf[first_attr..end].copy_from_slice(&self.attrs);
            self.buf[end..end + 4].copy_from_slice(&ATTR_END.to_le_bytes());
            let used = (end + 4) as u32;
            self.buf[0x18..0x1C].copy_from_slice(&used.to_le_bytes());
            self.buf[0x1C..0x20].copy_from_slice(&(RECORD as u32).to_le_bytes());

            // Stash each sector's real last two bytes in the array and
            // stamp the USN over them, mimicking what NTFS wrote.
            let usn: u16 = 0xBEEF;
            self.buf[0x30..0x32].copy_from_slice(&usn.to_le_bytes());
            for sector in 0..2usize {
                let footer = (sector + 1) * SECTOR as usize - 2;
                let original = [self.buf[footer], self.buf[footer + 1]];
                self.buf[0x32 + sector * 2..0x34 + sector * 2].copy_from_slice(&original);
                self.buf[footer..footer + 2].copy_from_slice(&usn.to_le_bytes());
            }
            self.buf
        }
    }

    fn resident_attr(attr_type: u32, value: &[u8], named: bool) -> Vec<u8> {
        let header = 0x18usize;
        let total = (header + value.len() + 7) & !7; // 8-byte aligned
        let mut a = vec![0u8; total];
        a[0x00..0x04].copy_from_slice(&attr_type.to_le_bytes());
        a[0x04..0x08].copy_from_slice(&(total as u32).to_le_bytes());
        a[0x08] = 0; // resident
        a[0x09] = if named { 4 } else { 0 };
        a[0x10..0x14].copy_from_slice(&(value.len() as u32).to_le_bytes());
        a[0x14..0x16].copy_from_slice(&(header as u16).to_le_bytes());
        a[header..header + value.len()].copy_from_slice(value);
        a
    }

    fn nonresident_attr(
        attr_type: u32,
        real: u64,
        allocated: u64,
        start_vcn: u64,
        named: bool,
    ) -> Vec<u8> {
        nonresident_attr_with_runs(attr_type, real, allocated, start_vcn, named, &[])
    }

    fn nonresident_attr_with_runs(
        attr_type: u32,
        real: u64,
        allocated: u64,
        start_vcn: u64,
        named: bool,
        runs: &[u8],
    ) -> Vec<u8> {
        let runlist_offset = 0x48usize;
        let total = ((runlist_offset + runs.len().max(1)) + 7) & !7;
        let mut a = vec![0u8; total];
        a[0x00..0x04].copy_from_slice(&attr_type.to_le_bytes());
        a[0x04..0x08].copy_from_slice(&(total as u32).to_le_bytes());
        a[0x08] = 1; // non-resident
        a[0x09] = if named { 4 } else { 0 };
        a[0x10..0x18].copy_from_slice(&start_vcn.to_le_bytes());
        a[0x20..0x22].copy_from_slice(&(runlist_offset as u16).to_le_bytes());
        a[0x28..0x30].copy_from_slice(&allocated.to_le_bytes());
        a[0x30..0x38].copy_from_slice(&real.to_le_bytes());
        a[runlist_offset..runlist_offset + runs.len()].copy_from_slice(runs);
        a
    }

    fn fixed_up(mut record: Vec<u8>) -> Vec<u8> {
        apply_fixups(&mut record, SECTOR).expect("fixups should apply");
        record
    }

    #[test]
    fn fixups_restore_the_bytes_ntfs_stole_from_sector_footers() {
        let mut raw = RecordBuilder::new().resident_data(b"hello").build();
        // The footers currently hold the USN, not the record's real bytes.
        assert_eq!(u16_at(&raw, SECTOR as usize - 2).unwrap(), 0xBEEF);
        apply_fixups(&mut raw, SECTOR).unwrap();
        assert_eq!(
            u16_at(&raw, SECTOR as usize - 2).unwrap(),
            0,
            "the saved original (zero padding here) should be back in place"
        );
    }

    #[test]
    fn a_torn_record_is_detected_rather_than_parsed_as_garbage() {
        let mut raw = RecordBuilder::new().resident_data(b"hello").build();
        // Simulate the second sector never reaching the disk: its footer
        // still holds an older USN.
        let footer = 2 * SECTOR as usize - 2;
        raw[footer..footer + 2].copy_from_slice(&0x1234u16.to_le_bytes());
        assert_eq!(
            apply_fixups(&mut raw, SECTOR),
            Err(NtfsError::FixupMismatch)
        );
    }

    #[test]
    fn parses_name_parent_and_mtime() {
        // 1970-01-01 + 3600s, expressed as FILETIME.
        let filetime = (11_644_473_600u64 + 3600) * 10_000_000;
        let raw = fixed_up(
            RecordBuilder::new()
                .standard_information(filetime)
                .file_name("report.pdf", 5, NAMESPACE_WIN32)
                .nonresident_data(9000, 12288, 0)
                .build(),
        );
        let r = parse(&raw).unwrap();
        assert!(r.in_use);
        assert!(!r.is_dir);
        assert_eq!(r.name.as_deref(), Some("report.pdf"));
        assert_eq!(r.parent, Some(5));
        assert_eq!(r.mtime, 3600);
        assert_eq!(r.size_logical, 9000);
        assert_eq!(r.size_alloc, 12288);
        assert_eq!(r.record_number, Some(42));
    }

    #[test]
    fn the_dos_short_name_never_wins_over_the_real_one() {
        // Real volumes carry both; taking the DOS entry would show
        // "PROGRA~1" instead of "Program Files".
        let raw = fixed_up(
            RecordBuilder::new()
                .file_name("PROGRA~1", 5, NAMESPACE_DOS)
                .file_name("Program Files", 5, NAMESPACE_WIN32)
                .build(),
        );
        assert_eq!(parse(&raw).unwrap().name.as_deref(), Some("Program Files"));

        // Order must not matter.
        let raw = fixed_up(
            RecordBuilder::new()
                .file_name("Program Files", 5, NAMESPACE_WIN32)
                .file_name("PROGRA~1", 5, NAMESPACE_DOS)
                .build(),
        );
        assert_eq!(parse(&raw).unwrap().name.as_deref(), Some("Program Files"));
    }

    #[test]
    fn a_dos_only_name_is_still_better_than_none() {
        let raw = fixed_up(
            RecordBuilder::new()
                .file_name("READ~1.TXT", 5, NAMESPACE_DOS)
                .build(),
        );
        assert_eq!(parse(&raw).unwrap().name.as_deref(), Some("READ~1.TXT"));
    }

    #[test]
    fn resident_data_reports_no_on_disk_size() {
        // A file small enough to live inside its MFT record occupies no
        // clusters of its own, which is what Windows reports too.
        let raw = fixed_up(
            RecordBuilder::new()
                .file_name("tiny.txt", 5, NAMESPACE_WIN32)
                .resident_data(b"12345")
                .build(),
        );
        let r = parse(&raw).unwrap();
        assert_eq!(r.size_logical, 5);
        assert_eq!(r.size_alloc, 0);
    }

    #[test]
    fn alternate_data_streams_add_to_the_owning_file() {
        let raw = fixed_up(
            RecordBuilder::new()
                .file_name("movie.mkv", 5, NAMESPACE_WIN32)
                .nonresident_data(1000, 4096, 0)
                .named_stream(500, 4096)
                .build(),
        );
        let r = parse(&raw).unwrap();
        assert_eq!(r.size_logical, 1500, "ADS bytes belong to the file");
        assert_eq!(r.size_alloc, 8192);
    }

    #[test]
    fn later_fragments_of_a_split_stream_do_not_double_count() {
        // Only the fragment at VCN 0 carries the stream's real totals.
        let raw = fixed_up(
            RecordBuilder::new()
                .file_name("huge.bin", 5, NAMESPACE_WIN32)
                .nonresident_data(100_000, 102_400, 0)
                .nonresident_data(100_000, 102_400, 64)
                .build(),
        );
        let r = parse(&raw).unwrap();
        assert_eq!(r.size_logical, 100_000);
        assert_eq!(r.size_alloc, 102_400);
    }

    #[test]
    fn directories_report_no_size_of_their_own() {
        let raw = fixed_up(
            RecordBuilder::new()
                .directory()
                .file_name("Windows", 5, NAMESPACE_WIN32)
                .nonresident_data(4096, 4096, 0)
                .build(),
        );
        let r = parse(&raw).unwrap();
        assert!(r.is_dir);
        assert_eq!(r.size_logical, 0);
        assert_eq!(r.size_alloc, 0);
    }

    #[test]
    fn unused_records_are_reported_without_walking_stale_attributes() {
        let raw = fixed_up(
            RecordBuilder::new()
                .not_in_use()
                .file_name("deleted.txt", 5, NAMESPACE_WIN32)
                .build(),
        );
        let r = parse(&raw).unwrap();
        assert!(!r.in_use);
        assert_eq!(r.name, None);
    }

    #[test]
    fn attribute_list_presence_is_reported() {
        let raw = fixed_up(
            RecordBuilder::new()
                .file_name("frag.bin", 5, NAMESPACE_WIN32)
                .attribute_list()
                .build(),
        );
        assert!(parse(&raw).unwrap().has_attribute_list);
    }

    #[test]
    fn malformed_records_error_or_stop_instead_of_panicking_or_looping() {
        // Too short to even hold a signature — which error it is doesn't
        // matter, only that it's an error.
        assert!(parse(&[]).is_err());
        // Long enough to check, and the signature is absent: unused MFT
        // space looks exactly like this.
        assert_eq!(parse(&[0u8; 1024]), Err(NtfsError::BadMagic));

        // A zero-length attribute would never advance the walk.
        let mut raw = fixed_up(
            RecordBuilder::new()
                .file_name("a.txt", 5, NAMESPACE_WIN32)
                .build(),
        );
        let first_attr = u16_at(&raw, 0x14).unwrap() as usize;
        raw[first_attr + 4..first_attr + 8].copy_from_slice(&0u32.to_le_bytes());
        let r = parse(&raw).expect("must return, not hang");
        assert_eq!(r.name, None);

        // An attribute claiming to run past the record must not be read.
        let mut raw = fixed_up(
            RecordBuilder::new()
                .file_name("a.txt", 5, NAMESPACE_WIN32)
                .build(),
        );
        raw[first_attr + 4..first_attr + 8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert!(parse(&raw).is_ok());

        // Truncated buffer.
        let raw = fixed_up(
            RecordBuilder::new()
                .file_name("a.txt", 5, NAMESPACE_WIN32)
                .build(),
        );
        assert!(
            parse(&raw[..64]).is_ok(),
            "short buffer clamps the walk rather than over-reading"
        );
    }

    #[test]
    fn the_mft_own_runlist_is_extractable() {
        // How the reader bootstraps: record 0 describes where the whole
        // table lives, so its unnamed $DATA runlist must come back intact.
        let runs = [0x21u8, 0x28, 0x34, 0x01, 0x00];
        let raw = fixed_up(RecordBuilder::new().data_with_runs(&runs).build());
        let extracted = unnamed_data_runlist(&raw)
            .unwrap()
            .expect("record has a $DATA runlist");
        assert_eq!(&extracted[..runs.len()], &runs);
        assert_eq!(
            crate::ntfs::runlist::parse(extracted).unwrap(),
            vec![crate::ntfs::runlist::Extent {
                lcn: 0x0134,
                clusters: 0x28
            }]
        );
    }

    #[test]
    fn a_record_without_nonresident_data_has_no_runlist() {
        let raw = fixed_up(RecordBuilder::new().resident_data(b"inline").build());
        assert_eq!(unnamed_data_runlist(&raw).unwrap(), None);
    }

    #[test]
    fn a_named_stream_runlist_is_not_mistaken_for_the_main_one() {
        let raw = fixed_up(RecordBuilder::new().named_stream(10, 4096).build());
        assert_eq!(unnamed_data_runlist(&raw).unwrap(), None);
    }

    #[test]
    fn fixup_rejects_arrays_that_do_not_fit_the_record() {
        let mut raw = RecordBuilder::new().build();
        raw[0x06..0x08].copy_from_slice(&999u16.to_le_bytes());
        assert!(apply_fixups(&mut raw, SECTOR).is_err());

        let mut raw = RecordBuilder::new().build();
        raw[0x06..0x08].copy_from_slice(&0u16.to_le_bytes());
        assert!(apply_fixups(&mut raw, SECTOR).is_err());
    }
}
