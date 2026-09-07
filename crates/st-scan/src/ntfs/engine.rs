//! Orchestration: stream the MFT off the volume, parse records in
//! parallel, and rebuild the directory tree from their parent pointers.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use rayon::prelude::*;
use st_core::{NodeFlags, NodeId, RawNode, TreeBuilder, ROOT};

use super::{boot, record, runlist, volume::VolumeReader};
use crate::{ScanProgress, ScanResult};

/// NTFS fixes the root directory at record 5.
const ROOT_RECORD: u32 = 5;

/// Read the table in large sequential bites — the whole point of this
/// engine is trading millions of small directory syscalls for a handful
/// of big reads.
const CHUNK_TARGET: usize = 8 * 1024 * 1024;

/// One MFT record's worth of extracted fields, indexed by record number.
///
/// Names live in a shared arena rather than a `String` per entry: at a
/// few million records the per-allocation overhead and cache pressure of
/// individual `String`s is worth avoiding, and the tree this feeds uses
/// the same representation anyway.
#[derive(Clone, Default)]
struct Entry {
    name_start: u32,
    name_len: u16,
    parent: u32,
    flags: NodeFlags,
    size_logical: u64,
    size_alloc: u64,
    mtime: i64,
    present: bool,
}

struct Index {
    names: String,
    entries: Vec<Entry>,
}

impl Index {
    fn name_of(&self, entry: &Entry) -> &str {
        let start = entry.name_start as usize;
        &self.names[start..start + entry.name_len as usize]
    }
}

/// Scan a whole NTFS volume by reading its Master File Table.
///
/// Requires the process to be elevated; without that the volume handle
/// can't be opened and this returns a permission error, which callers
/// treat as "use the directory walker instead".
pub fn scan_volume(
    drive_letter: char,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(ScanProgress),
) -> io::Result<ScanResult> {
    let start = Instant::now();
    let mut reader = VolumeReader::open(drive_letter)?;

    // Bootstrap read: 4096 bytes is a whole number of sectors for every
    // sector size a volume can have, so this is legal before we know
    // which one this volume uses.
    let mut boot_buf = vec![0u8; 4096];
    reader.read_exact_at(0, &mut boot_buf)?;
    let bpb = boot::parse(&boot_buf).map_err(to_io)?;
    reader.set_sector_size(bpb.bytes_per_sector);

    let mft_offset = bpb
        .mft_byte_offset()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MFT offset overflows"))?;

    // Record 0 is $MFT itself, and describes where the rest of the table
    // physically lives.
    let mut first = vec![0u8; bpb.bytes_per_record as usize];
    reader.read_exact_at(mft_offset, &mut first)?;
    record::apply_fixups(&mut first, bpb.bytes_per_sector).map_err(to_io)?;
    let runs = record::unnamed_data_runlist(&first)
        .map_err(to_io)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "$MFT has no data runs"))?;
    let extents = runlist::parse(runs).map_err(to_io)?;
    if extents.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "$MFT runlist is empty",
        ));
    }

    let index = read_and_parse(&reader, &bpb, &extents, cancel, start, &mut on_progress)?;
    let (tree, root) = build_tree(&index, drive_letter);

    Ok(ScanResult {
        tree,
        root,
        duration: start.elapsed(),
        denied_count: 0,
        engine: "NTFS MFT",
    })
}

fn read_and_parse(
    reader: &VolumeReader,
    bpb: &boot::BootSector,
    extents: &[runlist::Extent],
    cancel: &AtomicBool,
    start: Instant,
    on_progress: &mut impl FnMut(ScanProgress),
) -> io::Result<Index> {
    let record_size = bpb.bytes_per_record as usize;
    // Keep chunk reads a whole number of records so a record only
    // straddles a boundary when the volume's own extents force it.
    let chunk_size = (CHUNK_TARGET / record_size).max(1) * record_size;

    let total_bytes: u64 = extents
        .iter()
        .map(|e| e.clusters * bpb.bytes_per_cluster)
        .sum();
    let estimated_records = (total_bytes / record_size as u64) as usize;

    let mut index = Index {
        names: String::with_capacity(estimated_records * 16),
        entries: vec![Entry::default(); estimated_records + 1],
    };

    // Carries a partial record across a chunk (or extent) boundary, so a
    // record split by the volume's allocation still parses.
    let mut pending: Vec<u8> = Vec::with_capacity(chunk_size + record_size);
    let mut next_record: u64 = 0;
    let mut files_seen: u64 = 0;
    let mut bytes_seen: u64 = 0;
    let mut buf = vec![0u8; chunk_size];

    for extent in extents {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut physical = extent.lcn * bpb.bytes_per_cluster;
        let mut remaining = extent.clusters * bpb.bytes_per_cluster;

        while remaining > 0 {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let want = remaining.min(chunk_size as u64) as usize;
            let slice = &mut buf[..want];
            if reader.read_exact_at(physical, slice).is_err() {
                // A bad region shouldn't sink the whole scan; skip it and
                // keep the record numbering aligned.
                physical += want as u64;
                remaining -= want as u64;
                next_record += (want / record_size) as u64;
                continue;
            }
            physical += want as u64;
            remaining -= want as u64;

            pending.extend_from_slice(slice);
            let whole = pending.len() / record_size * record_size;
            if whole == 0 {
                continue;
            }

            // The expensive part — fixups and attribute walking — runs
            // across all cores; merging the results back is cheap.
            let parsed: Vec<(u64, record::FileRecord)> = pending[..whole]
                .par_chunks_mut(record_size)
                .enumerate()
                .filter_map(|(i, rec)| {
                    record::apply_fixups(rec, bpb.bytes_per_sector).ok()?;
                    let parsed = record::parse(rec).ok()?;
                    Some((next_record + i as u64, parsed))
                })
                .collect();

            for (number, parsed) in parsed {
                if merge(&mut index, number, parsed) {
                    files_seen += 1;
                }
            }
            bytes_seen += whole as u64;
            next_record += (whole / record_size) as u64;
            pending.drain(..whole);

            on_progress(ScanProgress {
                files_seen,
                bytes_seen,
                elapsed: start.elapsed(),
            });
        }
    }

    Ok(index)
}

/// Fold one parsed record into the index. Returns whether it counted as
/// a file, for progress reporting.
fn merge(index: &mut Index, number: u64, parsed: record::FileRecord) -> bool {
    // Extension records hold the overflow attributes of another record;
    // their contents belong to the base record, not to a row of their own.
    if !parsed.in_use || parsed.base_record != 0 {
        return false;
    }
    let (Some(name), Some(parent)) = (parsed.name, parsed.parent) else {
        return false;
    };
    let Ok(number) = u32::try_from(number) else {
        return false;
    };
    let Ok(parent) = u32::try_from(parent) else {
        return false;
    };

    if number as usize >= index.entries.len() {
        index.entries.resize(number as usize + 1, Entry::default());
    }

    let name_start = index.names.len() as u32;
    index.names.push_str(&name);
    let name_len = u16::try_from(name.len()).unwrap_or(u16::MAX);

    let mut flags = NodeFlags::empty();
    if parsed.is_dir {
        flags |= NodeFlags::DIR;
    }

    index.entries[number as usize] = Entry {
        name_start,
        name_len,
        parent,
        flags,
        size_logical: parsed.size_logical,
        size_alloc: parsed.size_alloc,
        mtime: parsed.mtime,
        present: true,
    };
    !parsed.is_dir
}

/// Rebuild the directory hierarchy and push it into a `TreeBuilder`.
///
/// Records arrive in table order, so a child is routinely seen before its
/// parent. Rather than sorting, this indexes children by parent (the same
/// CSR layout the tree itself uses) and then walks down from the root, so
/// every node is pushed after the parent it references — which is what
/// `TreeBuilder` requires.
fn build_tree(index: &Index, drive_letter: char) -> (st_core::Tree, NodeId) {
    let n = index.entries.len();
    let mut child_count = vec![0u32; n];
    for (i, entry) in index.entries.iter().enumerate() {
        if !entry.present || i as u32 == ROOT_RECORD {
            continue;
        }
        if (entry.parent as usize) < n {
            child_count[entry.parent as usize] += 1;
        }
    }

    let mut child_start = vec![0u32; n + 1];
    for i in 0..n {
        child_start[i + 1] = child_start[i] + child_count[i];
    }
    let mut cursor = child_start.clone();
    let mut children = vec![0u32; child_start[n] as usize];
    for (i, entry) in index.entries.iter().enumerate() {
        if !entry.present || i as u32 == ROOT_RECORD {
            continue;
        }
        let parent = entry.parent as usize;
        if parent < n {
            children[cursor[parent] as usize] = i as u32;
            cursor[parent] += 1;
        }
    }

    let mut builder = TreeBuilder::new();
    let root = builder.push(RawNode {
        parent: ROOT,
        name: format!("{}:\\", drive_letter.to_ascii_uppercase()),
        size_logical: 0,
        size_alloc: 0,
        mtime: 0,
        flags: NodeFlags::DIR,
    });

    let mut node_of = vec![u32::MAX; n];
    // A volume with no reachable root record yields just the drive node
    // rather than indexing past the end of an empty index.
    let mut stack = Vec::new();
    if (ROOT_RECORD as usize) < n {
        node_of[ROOT_RECORD as usize] = root;
        stack.push(ROOT_RECORD);
    }
    while let Some(rec) = stack.pop() {
        let parent_node = node_of[rec as usize];
        let start = child_start[rec as usize] as usize;
        let end = child_start[rec as usize + 1] as usize;
        for &child in &children[start..end] {
            let entry = &index.entries[child as usize];
            let node = builder.push(RawNode {
                parent: parent_node,
                name: index.name_of(entry).to_string(),
                size_logical: entry.size_logical,
                size_alloc: entry.size_alloc,
                mtime: entry.mtime,
                flags: entry.flags,
            });
            node_of[child as usize] = node;
            if entry.flags.contains(NodeFlags::DIR) {
                stack.push(child);
            }
        }
    }

    (builder.finalize(), root)
}

fn to_io(e: super::NtfsError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
