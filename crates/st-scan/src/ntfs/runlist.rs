//! Data-run decoding: turns a non-resident attribute's compact runlist
//! into the list of on-disk extents it occupies.
//!
//! This is how we find where the MFT physically lives — `$MFT`'s own
//! `$DATA` attribute carries the runlist for the whole table, and the
//! reader then walks those extents in large sequential reads.
//!
//! Encoding: each run starts with a header byte whose low nibble is the
//! byte-width of a run length and whose high nibble is the byte-width of
//! an LCN delta. A zero header ends the list. The delta is *signed* and
//! relative to the previous run's start, so runs can move backwards
//! across the disk. A zero-width delta marks a sparse run — a hole with
//! no clusters backing it.

use super::{slice_at, u8_at, NtfsError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    /// Starting logical cluster number on the volume.
    pub lcn: u64,
    pub clusters: u64,
}

/// Decode a runlist. Sparse runs are skipped (they hold no data to
/// read), but their length still advances the VCN, which matters only to
/// callers mapping VCN→LCN; for the MFT read path we just need the
/// backed extents in order.
pub fn parse(buf: &[u8]) -> Result<Vec<Extent>> {
    let mut extents = Vec::new();
    let mut offset = 0usize;
    let mut current_lcn: i64 = 0;

    loop {
        let header = u8_at(buf, offset)?;
        if header == 0 {
            return Ok(extents);
        }
        offset += 1;

        let len_size = (header & 0x0F) as usize;
        let off_size = (header >> 4) as usize;
        if len_size == 0 || len_size > 8 || off_size > 8 {
            return Err(NtfsError::BadField("run header nibble"));
        }

        let clusters = read_unsigned(buf, offset, len_size)?;
        offset += len_size;

        if off_size == 0 {
            // Sparse run: no backing clusters, nothing to read.
            offset += off_size;
            continue;
        }

        let delta = read_signed(buf, offset, off_size)?;
        offset += off_size;

        current_lcn = current_lcn
            .checked_add(delta)
            .ok_or(NtfsError::BadField("runlist LCN overflow"))?;
        if current_lcn < 0 {
            return Err(NtfsError::BadField("negative LCN"));
        }

        extents.push(Extent {
            lcn: current_lcn as u64,
            clusters,
        });
    }
}

fn read_unsigned(buf: &[u8], offset: usize, size: usize) -> Result<u64> {
    let bytes = slice_at(buf, offset, size)?;
    let mut value = 0u64;
    for (i, &b) in bytes.iter().enumerate() {
        value |= (b as u64) << (i * 8);
    }
    Ok(value)
}

/// Little-endian, sign-extended from its top byte — the delta may be
/// negative when a later extent sits earlier on the disk.
fn read_signed(buf: &[u8], offset: usize, size: usize) -> Result<i64> {
    let bytes = slice_at(buf, offset, size)?;
    let mut value = 0u64;
    for (i, &b) in bytes.iter().enumerate() {
        value |= (b as u64) << (i * 8);
    }
    let sign_bit = 1u64 << (size * 8 - 1);
    if value & sign_bit != 0 {
        // Fill the unused high bytes with ones to sign-extend.
        let mask = !0u64 << (size * 8);
        value |= mask;
    }
    Ok(value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_runlist_terminates_immediately() {
        assert_eq!(parse(&[0x00]).unwrap(), vec![]);
    }

    #[test]
    fn single_run_decodes() {
        // 0x21: 1-byte length, 2-byte offset. length 0x28, LCN 0x0134.
        let buf = [0x21, 0x28, 0x34, 0x01, 0x00];
        assert_eq!(
            parse(&buf).unwrap(),
            vec![Extent {
                lcn: 0x0134,
                clusters: 0x28
            }]
        );
    }

    #[test]
    fn subsequent_offsets_are_deltas_from_the_previous_run() {
        // Run 1: len 0x10 at LCN 0x20. Run 2: len 0x10 at +0x10 => 0x30.
        let buf = [0x11, 0x10, 0x20, 0x11, 0x10, 0x10, 0x00];
        assert_eq!(
            parse(&buf).unwrap(),
            vec![
                Extent {
                    lcn: 0x20,
                    clusters: 0x10
                },
                Extent {
                    lcn: 0x30,
                    clusters: 0x10
                }
            ]
        );
    }

    #[test]
    fn negative_delta_moves_backwards_on_disk() {
        // Run 1 at LCN 0x60, run 2 at delta -0x20 => 0x40. A real
        // fragmented file does this constantly.
        let buf = [0x11, 0x10, 0x60, 0x11, 0x10, 0xE0, 0x00];
        assert_eq!(
            parse(&buf).unwrap(),
            vec![
                Extent {
                    lcn: 0x60,
                    clusters: 0x10
                },
                Extent {
                    lcn: 0x40,
                    clusters: 0x10
                }
            ]
        );
    }

    #[test]
    fn sparse_run_contributes_no_extent() {
        // 0x01: 1-byte length, 0-byte offset => sparse hole of 0x30.
        let buf = [0x11, 0x10, 0x20, 0x01, 0x30, 0x11, 0x10, 0x10, 0x00];
        let extents = parse(&buf).unwrap();
        assert_eq!(
            extents,
            vec![
                Extent {
                    lcn: 0x20,
                    clusters: 0x10
                },
                Extent {
                    lcn: 0x30,
                    clusters: 0x10
                }
            ],
            "a hole has no clusters to read, but must not disturb the following delta"
        );
    }

    #[test]
    fn wide_length_and_offset_fields_decode() {
        // 0x42: 2-byte length, 4-byte offset.
        let buf = [0x42, 0x00, 0x10, 0x00, 0x00, 0x01, 0x00, 0x00];
        assert_eq!(
            parse(&buf).unwrap(),
            vec![Extent {
                lcn: 0x0001_0000,
                clusters: 0x1000
            }]
        );
    }

    #[test]
    fn truncated_runlist_errors_instead_of_panicking() {
        assert!(parse(&[0x21, 0x28]).is_err(), "offset field is cut off");
        assert!(parse(&[0x21]).is_err());
        assert!(parse(&[]).is_err(), "no terminator and no data");
    }

    #[test]
    fn nonsense_header_nibbles_are_rejected() {
        // Zero length-width with a non-zero header is not decodable.
        assert!(parse(&[0x10, 0x00]).is_err());
    }
}
