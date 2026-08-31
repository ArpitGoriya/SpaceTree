//! Portable parallel directory walker — Engine B from the project plan.
//!
//! Built against `std::fs` rather than raw platform APIs, so it compiles
//! and is fully testable on any host (this crate is developed on Linux;
//! see docs/PLAN.md's Environment note). That also makes it exactly the
//! engine the plan calls for on non-NTFS volumes (exFAT, ReFS, network
//! shares, USB) once Engine A (the Windows MFT reader) exists, and a
//! genuine macOS/Linux backend beyond that. What it does *not* attempt
//! is the Win32-specific speedup in the plan's Engine B section
//! (`FindFirstFileExW` avoiding a second stat per file) — that requires
//! the `windows` crate and a Windows target to write *and verify*, so
//! it's left for when this can be built somewhere that can compile and
//! run it.
//!
//! Parallelism model: I/O (readdir + per-entry metadata) runs on
//! rayon's work-stealing pool, one task per directory. Tree construction
//! stays single-threaded — a directory's `NodeId` must exist before its
//! children can be pushed with that id as their parent, and enforcing
//! that across threads is exactly what a single collector avoids having
//! to coordinate. Workers send raw entries back over a channel; the
//! calling thread is the sole collector, pushing into one `TreeBuilder`
//! and dispatching a new worker per subdirectory it just assigned an id.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use crossbeam_channel as chan;
use std::time::{Duration, Instant, UNIX_EPOCH};

use st_core::{NodeFlags, NodeId, RawNode, Tree, TreeBuilder, ROOT};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Default)]
pub struct ScanProgress {
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub struct ScanResult {
    pub tree: Tree,
    pub root: NodeId,
    pub duration: Duration,
    /// Directories that could not be read (permissions, races with
    /// deletion, etc.) — surfaced as a count so the UI can show an
    /// "N folders not readable" chip rather than failing the whole scan.
    pub denied_count: u64,
}

/// (device, inode) — identifies a hard link's underlying data on Unix.
/// Unix-only: Windows hard-link identity needs
/// `GetFileInformationByHandle`, which isn't exposed through
/// `std::fs::Metadata` and belongs with the future Win32 backend instead
/// of being half-implemented against APIs this session can't verify.
#[cfg(unix)]
type InodeKey = (u64, u64);

/// Walks `root_path` and every descendant, producing a finalized [`Tree`].
///
/// `cancel` is checked between directories (not per-file — matching the
/// plan's "AtomicBool checked per chunk"); once set, in-flight readdirs
/// finish but no further directories are dispatched, and the partial
/// tree found so far is returned rather than an error, since a
/// deliberately cancelled scan isn't a failure.
///
/// `on_progress` fires on the calling thread only, at ~10 Hz, never once
/// per file — matching the plan's IPC rule against overwhelming a UI
/// with per-file events.
pub fn scan(
    root_path: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(ScanProgress) + Send,
) -> std::io::Result<ScanResult> {
    let start = Instant::now();

    let root_meta = std::fs::symlink_metadata(root_path)?;
    if !root_meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "scan root is not a directory",
        ));
    }

    let mut builder = TreeBuilder::new();
    let root_id = builder.push(RawNode {
        parent: ROOT,
        name: display_name(root_path),
        size_logical: 0,
        size_alloc: 0,
        mtime: 0,
        flags: NodeFlags::DIR,
    });

    let files_seen = AtomicU64::new(0);
    let bytes_seen = AtomicU64::new(0);
    let denied = AtomicU64::new(0);
    #[cfg(unix)]
    let seen_inodes: Mutex<HashSet<InodeKey>> = Mutex::new(HashSet::new());

    let (tx, rx) = chan::unbounded::<DirMsg>();
    let ctx = WorkerCtx {
        files_seen: &files_seen,
        bytes_seen: &bytes_seen,
        denied: &denied,
        #[cfg(unix)]
        seen_inodes: &seen_inodes,
        cancel,
    };

    rayon::scope(|s| {
        // Dispatch decisions (which subdirectory gets scanned next) are
        // made entirely by this collector loop, so it must keep a live
        // Sender to clone from for as long as new work can still appear
        // — the channel can never be allowed to close itself early. Its
        // place is instead this plain counter: every dispatch increments
        // it, every received-and-processed message decrements it, and
        // the loop ends exactly when it returns to zero. Untouched by
        // any worker thread, so no atomics needed.
        let mut outstanding: u64 = 1;
        spawn_dir_scan(s, root_path.to_path_buf(), root_id, tx.clone(), ctx);

        let mut last_tick = Instant::now();
        while outstanding > 0 {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let msg = match rx.recv_timeout(PROGRESS_INTERVAL) {
                Ok(msg) => msg,
                Err(chan::RecvTimeoutError::Timeout) => {
                    on_progress(ScanProgress {
                        files_seen: files_seen.load(Ordering::Relaxed),
                        bytes_seen: bytes_seen.load(Ordering::Relaxed),
                        elapsed: start.elapsed(),
                    });
                    last_tick = Instant::now();
                    continue;
                }
                Err(chan::RecvTimeoutError::Disconnected) => break,
            };
            outstanding -= 1;

            if msg.access_denied {
                builder.mark(msg.parent_id, NodeFlags::ACCESS_DENIED);
            }
            for entry in msg.entries {
                let child_id = builder.push(RawNode {
                    parent: msg.parent_id,
                    name: entry.name,
                    size_logical: entry.size_logical,
                    size_alloc: entry.size_alloc,
                    mtime: entry.mtime,
                    flags: entry.flags,
                });
                if let Some(child_path) = entry.recurse_into {
                    outstanding += 1;
                    spawn_dir_scan(s, child_path, child_id, tx.clone(), ctx);
                }
            }
            if last_tick.elapsed() >= PROGRESS_INTERVAL {
                on_progress(ScanProgress {
                    files_seen: files_seen.load(Ordering::Relaxed),
                    bytes_seen: bytes_seen.load(Ordering::Relaxed),
                    elapsed: start.elapsed(),
                });
                last_tick = Instant::now();
            }
        }
    });

    Ok(ScanResult {
        tree: builder.finalize(),
        root: root_id,
        duration: start.elapsed(),
        denied_count: denied.load(Ordering::Relaxed),
    })
}

fn display_name(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        // Drive/filesystem roots (`C:\`, `/`) have no file_name component;
        // their display form and their "name" are the same string.
        None => path.display().to_string(),
    }
}

struct Entry {
    name: String,
    size_logical: u64,
    size_alloc: u64,
    mtime: i64,
    flags: NodeFlags,
    /// `Some(path)` when this entry is a real directory the collector
    /// should dispatch a worker for; `None` for files and for symlinks
    /// (which are recorded but never traversed).
    recurse_into: Option<PathBuf>,
}

struct DirMsg {
    parent_id: NodeId,
    entries: Vec<Entry>,
    access_denied: bool,
}

#[derive(Clone, Copy)]
struct WorkerCtx<'s> {
    files_seen: &'s AtomicU64,
    bytes_seen: &'s AtomicU64,
    denied: &'s AtomicU64,
    #[cfg(unix)]
    seen_inodes: &'s Mutex<HashSet<InodeKey>>,
    cancel: &'s AtomicBool,
}

fn spawn_dir_scan<'s>(
    scope: &rayon::Scope<'s>,
    dir_path: PathBuf,
    parent_id: NodeId,
    tx: chan::Sender<DirMsg>,
    ctx: WorkerCtx<'s>,
) {
    scope.spawn(move |s| {
        if ctx.cancel.load(Ordering::Relaxed) {
            return;
        }

        let read_dir = match std::fs::read_dir(&dir_path) {
            Ok(rd) => rd,
            Err(_) => {
                ctx.denied.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(DirMsg {
                    parent_id,
                    entries: Vec::new(),
                    access_denied: true,
                });
                return;
            }
        };

        let mut entries = Vec::new();
        for dirent in read_dir {
            let dirent = match dirent {
                Ok(d) => d,
                Err(_) => {
                    ctx.denied.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            // `DirEntry::metadata` does not follow symlinks, matching
            // the "never traverse a reparse point" policy this shares
            // with the plan's Win32 walker.
            let meta = match dirent.metadata() {
                Ok(m) => m,
                Err(_) => {
                    ctx.denied.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };

            let is_symlink = meta.is_symlink();
            let looks_like_dir = meta.is_dir();
            let is_real_dir = looks_like_dir && !is_symlink;

            let mut flags = NodeFlags::empty();
            if looks_like_dir {
                flags |= NodeFlags::DIR;
            }
            if is_symlink {
                flags |= NodeFlags::REPARSE;
            }

            // Symlinks/junctions contribute 0 bytes (never followed, so
            // their target's size is never ours to report) — see the
            // size-semantics table in docs/PLAN.md. Directories carry no
            // "own" size either; Tree's rollup supplies subtree totals.
            //
            // Known, intentional consequence: a directory's own disk
            // blocks (its entry table — typically ~4 KiB, confirmed by
            // comparing against `du` on real trees during development)
            // aren't counted anywhere, so a full-tree on-disk total
            // reads a little under `du -s` — proportional to folder
            // count, not a bug. Logical size matches `du --apparent-size`
            // exactly, since that gap doesn't apply to it.
            let (mut size_logical, mut size_alloc) = if is_symlink || looks_like_dir {
                (0, 0)
            } else {
                (meta.len(), alloc_size(&meta))
            };

            #[cfg(unix)]
            if !looks_like_dir && !is_symlink {
                if let Some(key) = hardlink_key(&meta) {
                    let mut seen = ctx.seen_inodes.lock().unwrap();
                    if !seen.insert(key) {
                        // Already counted at its first-seen path: keep the
                        // node (so the link is still listed) but zero its
                        // bytes so rollup doesn't double-count them.
                        flags |= NodeFlags::HARDLINK_DUP;
                        size_logical = 0;
                        size_alloc = 0;
                    }
                }
            }

            let name = dirent.file_name().to_string_lossy().into_owned();

            if !looks_like_dir {
                ctx.files_seen.fetch_add(1, Ordering::Relaxed);
                ctx.bytes_seen.fetch_add(size_logical, Ordering::Relaxed);
            }

            let recurse_into = if is_real_dir {
                Some(dir_path.join(&name))
            } else {
                None
            };
            let mtime = mtime_secs(&meta);

            entries.push(Entry {
                name,
                size_logical,
                size_alloc,
                mtime,
                flags,
                recurse_into,
            });
        }

        let _ = tx.send(DirMsg {
            parent_id,
            entries,
            access_denied: false,
        });
        let _ = s; // nested scope handle; workers here don't spawn sub-scopes of their own
    });
}

fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn hardlink_key(meta: &std::fs::Metadata) -> Option<InodeKey> {
    use std::os::unix::fs::MetadataExt;
    if meta.nlink() > 1 {
        Some((meta.dev(), meta.ino()))
    } else {
        None
    }
}

#[cfg(unix)]
fn alloc_size(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn alloc_size(meta: &std::fs::Metadata) -> u64 {
    // No portable on-disk-allocation field outside std::fs on this
    // platform; report logical size rather than silently claiming an
    // on-disk number this build can't actually compute. The real fix —
    // NTFS's `allocated_size` via the Win32 backend — is future work
    // that must be built and verified on Windows, not guessed at here.
    meta.len()
}
