//! In-memory state for the currently loaded scan. Owned by Tauri's
//! managed state (one instance, `Mutex`-guarded) rather than sent
//! anywhere — the whole point of the IPC design in docs/PLAN.md is that
//! the tree never crosses into the webview; commands query windows of
//! it instead.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use st_core::{NodeId, Tree, VolumeInfo};

pub struct ScanState {
    pub tree: Tree,
    pub root: NodeId,
    pub volume: Option<VolumeInfo>,
    pub engine: String,
    pub duration: Duration,
    pub scanned_at: String,
}

#[derive(Default)]
pub struct AppState {
    pub scan: Mutex<Option<ScanState>>,
    /// The active scan's cancel flag, if a scan is currently running.
    /// `start_scan` installs a fresh one and clears it when done;
    /// `cancel_scan` flips whatever is installed, if anything.
    pub cancel: Mutex<Option<Arc<AtomicBool>>>,
}
