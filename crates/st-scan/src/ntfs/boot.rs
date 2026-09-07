//! NTFS boot sector (BPB) parsing — the entry point that tells us where
//! the MFT lives and how big its records are.

use super::{slice_at, u16_at, u64_at, u8_at, NtfsError, Result};

const OEM_ID: &[u8] = b"NTFS    ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootSector {
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub bytes_per_cluster: u64,
    /// Logical cluster number where the `$MFT` file begins.
    pub mft_lcn: u64,
    /// Size of one MFT FILE record. Almost always 1024.
    pub bytes_per_record: u32,
    pub total_sectors: u64,
}

impl BootSector {
    pub fn mft_byte_offset(&self) -> Option<u64> {
        self.mft_lcn.checked_mul(self.bytes_per_cluster)
    }
}

/// Parse an NTFS boot sector. Rejects anything whose geometry couldn't
/// describe a real volume, so a non-NTFS partition (or a garbage read)
/// fails here rather than producing nonsense offsets that would send the
/// reader chasing arbitrary disk locations.
pub fn parse(buf: &[u8]) -> Result<BootSector> {
    if slice_at(buf, 0x03, 8)? != OEM_ID {
        return Err(NtfsError::BadMagic);
    }

    let bytes_per_sector = u16_at(buf, 0x0B)? as u32;
    if !(256..=4096).contains(&bytes_per_sector) || !bytes_per_sector.is_power_of_two() {
        return Err(NtfsError::BadField("bytes per sector"));
    }

    // Usually a small count, but values above 0x80 are an exponent
    // encoding (2^(256-n)) used by volumes with very large clusters.
    let raw_spc = u8_at(buf, 0x0D)?;
    let sectors_per_cluster = if raw_spc > 0x80 {
        let shift = 256u32 - raw_spc as u32;
        if shift > 31 {
            return Err(NtfsError::BadField("sectors per cluster exponent"));
        }
        1u32 << shift
    } else if raw_spc == 0 {
        return Err(NtfsError::BadField("sectors per cluster"));
    } else {
        raw_spc as u32
    };

    let bytes_per_cluster = bytes_per_sector as u64 * sectors_per_cluster as u64;
    let total_sectors = u64_at(buf, 0x28)?;
    let mft_lcn = u64_at(buf, 0x30)?;

    // Positive means "this many clusters"; negative is a log2 byte size.
    // The negative form is what real volumes use (-10 → 1024 bytes),
    // since a record is normally smaller than a cluster.
    let raw_frs = u8_at(buf, 0x40)? as i8;
    let bytes_per_record = if raw_frs > 0 {
        (raw_frs as u64)
            .checked_mul(bytes_per_cluster)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(NtfsError::BadField("bytes per record"))?
    } else {
        let shift = raw_frs.unsigned_abs() as u32;
        if !(9..=20).contains(&shift) {
            return Err(NtfsError::BadField("record size exponent"));
        }
        1u32 << shift
    };

    // A record must at least hold its own header, and must be a whole
    // number of sectors or fixup application below makes no sense.
    if bytes_per_record < 48 || bytes_per_record % bytes_per_sector != 0 {
        return Err(NtfsError::BadField("bytes per record"));
    }

    Ok(BootSector {
        bytes_per_sector,
        sectors_per_cluster,
        bytes_per_cluster,
        mft_lcn,
        bytes_per_record,
        total_sectors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A boot sector with the geometry of a typical NTFS volume:
    /// 512-byte sectors, 8 sectors per cluster (4 KiB clusters), MFT at
    /// cluster 786432, 1024-byte records.
    fn sample_boot() -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[0x03..0x0B].copy_from_slice(OEM_ID);
        b[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
        b[0x0D] = 8;
        b[0x28..0x30].copy_from_slice(&2_000_000u64.to_le_bytes());
        b[0x30..0x38].copy_from_slice(&786_432u64.to_le_bytes());
        b[0x40] = (-10i8) as u8;
        b
    }

    #[test]
    fn parses_a_typical_volume() {
        let bs = parse(&sample_boot()).unwrap();
        assert_eq!(bs.bytes_per_sector, 512);
        assert_eq!(bs.sectors_per_cluster, 8);
        assert_eq!(bs.bytes_per_cluster, 4096);
        assert_eq!(bs.mft_lcn, 786_432);
        assert_eq!(bs.bytes_per_record, 1024);
        assert_eq!(bs.mft_byte_offset(), Some(786_432 * 4096));
    }

    #[test]
    fn rejects_a_non_ntfs_volume() {
        let mut b = sample_boot();
        b[0x03..0x0B].copy_from_slice(b"MSDOS5.0");
        assert_eq!(parse(&b), Err(NtfsError::BadMagic));
    }

    #[test]
    fn positive_record_size_is_read_as_clusters() {
        let mut b = sample_boot();
        b[0x40] = 1; // 1 cluster per record => 4096 bytes
        assert_eq!(parse(&b).unwrap().bytes_per_record, 4096);
    }

    #[test]
    fn large_cluster_exponent_encoding_is_handled() {
        let mut b = sample_boot();
        b[0x0D] = 0xF4; // 2^(256-244) = 4096 sectors per cluster
        b[0x40] = (-10i8) as u8;
        let bs = parse(&b).unwrap();
        assert_eq!(bs.sectors_per_cluster, 4096);
        assert_eq!(bs.bytes_per_cluster, 512 * 4096);
    }

    #[test]
    fn rejects_implausible_geometry_rather_than_computing_garbage_offsets() {
        let mut zero_sector = sample_boot();
        zero_sector[0x0B..0x0D].copy_from_slice(&0u16.to_le_bytes());
        assert!(parse(&zero_sector).is_err());

        let mut odd_sector = sample_boot();
        odd_sector[0x0B..0x0D].copy_from_slice(&513u16.to_le_bytes());
        assert!(
            parse(&odd_sector).is_err(),
            "sector size must be a power of two"
        );

        let mut zero_spc = sample_boot();
        zero_spc[0x0D] = 0;
        assert!(parse(&zero_spc).is_err());

        let mut tiny_record = sample_boot();
        tiny_record[0x40] = (-5i8) as u8; // 32 bytes, smaller than the header
        assert!(parse(&tiny_record).is_err());
    }

    #[test]
    fn truncated_buffer_errors_instead_of_panicking() {
        assert!(parse(&[]).is_err());
        assert!(parse(&sample_boot()[..0x20]).is_err());
    }
}
