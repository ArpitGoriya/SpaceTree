//! In-memory state for the currently loaded scan. Owned by Tauri's
//! managed state (one instance, `Mutex`-guarded) rather than sent
//! anywhere — the whole point of the IPC design in docs/PLAN.md is that
//! the tree never crosses into the webview; commands query windows of
//! it instead.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use st_core::{NodeId, Tree, VolumeInfo};

pub struct ScanState {
    pub tree: Tree,
    pub root: NodeId,
    /// The absolute path the scan was started from.
    ///
    /// Kept because `Tree::path` cannot reconstruct it: that walks node
    /// *names* up to the scan root, so scanning `D:\Games` yields
    /// `Games\Steam\...` — a relative string that would resolve against
    /// the process's working directory. Fine for display, catastrophic
    /// for a file operation, so anything touching the filesystem resolves
    /// through [`ScanState::absolute_path`] instead.
    pub root_path: PathBuf,
    pub volume: Option<VolumeInfo>,
    pub engine: String,
    pub duration: Duration,
    pub scanned_at: String,
}

impl ScanState {
    /// The real filesystem path of `id`, or `None` if the id isn't in
    /// this tree.
    ///
    /// Built by collecting names from `id` up to the scan root and
    /// joining them onto `root_path`, so the result is absolute whatever
    /// the scan root was, and correctly separated on every platform —
    /// `Path::join` also avoids the doubled separator that string
    /// concatenation produces after a drive root like `C:\`.
    pub fn absolute_path(&self, id: NodeId) -> Option<PathBuf> {
        if id != self.root && self.tree.parent_of(id).is_none() {
            return None;
        }
        let mut parts: Vec<&str> = Vec::new();
        let mut cursor = id;
        while cursor != self.root {
            parts.push(self.tree.name(cursor));
            cursor = self.tree.parent_of(cursor)?;
        }
        parts.reverse();
        let mut path = self.root_path.clone();
        for part in parts {
            path.push(part);
        }
        Some(path)
    }

    /// Whether `path` is inside (or is) the scanned root.
    ///
    /// Every filesystem action checks this before touching anything: the
    /// node id arrives from the webview, and a delete is not something to
    /// perform on a path that resolved somewhere unexpected.
    pub fn is_within_root(&self, path: &Path) -> bool {
        path.starts_with(&self.root_path)
    }
}

#[derive(Default)]
pub struct AppState {
    pub scan: Mutex<Option<ScanState>>,
    /// The assistant's cancel flag while a turn is in flight, so the
    /// Stop button can interrupt a long answer.
    pub ai_cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// The active scan's cancel flag, if a scan is currently running.
    /// `start_scan` installs a fresh one and clears it when done;
    /// `cancel_scan` flips whatever is installed, if anything.
    pub cancel: Mutex<Option<Arc<AtomicBool>>>,
}
