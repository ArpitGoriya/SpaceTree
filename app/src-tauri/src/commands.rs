//! IPC command surface. Every command that reads scan results locks
//! `AppState::scan` just long enough to read what it needs — the tree
//! itself never leaves this process; see docs/PLAN.md's "critical IPC
//! rule" for why (a multi-million-node tree serialized to JSON would
//! stall the webview).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{Emitter, State};

use st_core::export::{export_markdown, ScanMeta};
use st_core::treemap::{layout_children_aggregated, Rect, TreemapCell};
use st_core::{search as core_search, NodeId, Tree};
use st_scan::{scan_auto, ScanPhase};

use crate::dto::{
    ExportOptionsDto, FastScanStatusDto, HeaderDto, NodeInfoDto, RectDto, RowDto, ScanProgressDto,
    SearchHitDto, VolumeDto,
};
use crate::state::{AppState, ScanState};
use crate::volumes;

/// Smallest rect the treemap bothers to draw, in CSS px². Below roughly
/// this a rect is swallowed by its own 1px gutter and can carry no label,
/// so it is folded into the "N smaller items" cell instead of being
/// shipped over IPC and hit-tested for nothing.
const MIN_RECT_AREA_PX: f64 = 120.0;

/// Hard ceiling on rects per layout. A folder can have tens of thousands
/// of children and this runs again on every resize frame, so the payload
/// needs a bound that doesn't depend on how the sizes happen to fall.
const MAX_RECTS: usize = 250;

/// `/` on every platform this can actually run on today; kept as one
/// named constant rather than scattering `cfg!(windows)` checks, and
/// easy to widen if a portable macOS/Windows path ever needs `\`.
fn path_sep() -> &'static str {
    if cfg!(windows) {
        "\\"
    } else {
        "/"
    }
}

#[tauri::command]
pub fn list_volumes() -> Vec<VolumeDto> {
    volumes::list_volumes()
}

#[tauri::command]
pub async fn pick_folder(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = tx.send(path);
    });
    // The dialog plugin's callback runs on the main thread; blocking a
    // spawned-off async command's own thread for it is fine and simple.
    tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .ok()
        .flatten()
        .map(|p| p.to_string())
}

fn now_string() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Intentionally simple (no timezone/locale formatting deps): a raw
    // Unix timestamp is unambiguous and the frontend can render it
    // however it likes with `Intl.DateTimeFormat`.
    format!("epoch:{secs}")
}

#[tauri::command]
pub async fn start_scan(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<HeaderDto, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());

    let root_path = std::path::PathBuf::from(&path);
    let volume_hint = st_core::volume::query(&root_path).ok();

    let progress_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        scan_auto(&root_path, &cancel, move |p| {
            let _ = progress_app.emit(
                "scan_progress",
                ScanProgressDto {
                    files_seen: p.files_seen,
                    bytes_seen: p.bytes_seen,
                    elapsed_ms: p.elapsed.as_millis() as u64,
                    engine: p.engine.to_string(),
                    phase: match p.phase {
                        ScanPhase::Indexing => "indexing",
                        ScanPhase::BuildingTree => "buildingTree",
                    }
                    .to_string(),
                },
            );
        })
    })
    .await
    .map_err(|e| format!("scan task panicked: {e}"))?
    .map_err(|e| format!("scan failed: {e}"))?;

    *state.cancel.lock().unwrap() = None;

    let tree = result.tree;
    let root = result.root;
    let indexed_alloc = tree.subtree_alloc(root);
    let indexed_files = tree.file_count(root);
    let indexed_folders =
        (tree.descendants(root).filter(|&id| tree.is_dir(id)).count() as u32).saturating_sub(1);

    let header = HeaderDto {
        root_id: root,
        root_name: tree.name(root).to_string(),
        root_path: tree.path(root, path_sep()),
        engine: result.engine.to_string(),
        duration_ms: result.duration.as_millis() as u64,
        scanned_at: now_string(),
        denied_count: result.denied_count,
        volume: volume_hint
            .as_ref()
            .map(|v| VolumeDto::new(path.clone(), path.clone(), v)),
        indexed_files,
        indexed_folders,
        indexed_logical: tree.subtree_logical(root),
        indexed_alloc,
    };

    *state.scan.lock().unwrap() = Some(ScanState {
        tree,
        root,
        root_path: std::path::PathBuf::from(&path),
        volume: volume_hint,
        engine: header.engine.clone(),
        duration: result.duration,
        scanned_at: header.scanned_at.clone(),
    });

    Ok(header)
}

/// Whether `path` could be scanned by the fast NTFS engine, and whether
/// this process already has the rights to do so. Lets the launcher offer
/// "restart as administrator for a much faster scan" instead of silently
/// taking the slow path.
#[tauri::command]
pub fn fast_scan_status(path: String) -> FastScanStatusDto {
    let path = std::path::PathBuf::from(path);
    FastScanStatusDto {
        available: st_scan::can_use_fast_engine(&path),
        elevated: elevated_now(),
    }
}

/// Relaunch elevated so the MFT engine can open the raw volume. Returns
/// false when the user declines the prompt, which is a choice rather than
/// an error — the app keeps working on the slower engine.
#[tauri::command]
pub fn request_elevation() -> Result<bool, String> {
    #[cfg(windows)]
    {
        st_scan::ntfs::elevation::relaunch_elevated(&[]).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

fn elevated_now() -> bool {
    #[cfg(windows)]
    {
        st_scan::ntfs::elevation::is_elevated()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) {
    if let Some(flag) = state.cancel.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

fn with_tree<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&Tree, NodeId) -> T,
) -> Result<T, String> {
    let guard = state.scan.lock().unwrap();
    let scan = guard.as_ref().ok_or("no scan loaded")?;
    Ok(f(&scan.tree, scan.root))
}

fn row_dto(tree: &Tree, id: NodeId, parent_size: u64, use_alloc: bool) -> RowDto {
    let flags = tree.flags(id);
    let size_alloc = tree.subtree_alloc(id);
    let size_logical = tree.subtree_logical(id);
    let this_size = if use_alloc { size_alloc } else { size_logical };
    RowDto {
        id,
        name: tree.name(id).to_string(),
        is_dir: tree.is_dir(id),
        is_symlink: flags.contains(st_core::NodeFlags::REPARSE),
        is_hardlink_dup: flags.contains(st_core::NodeFlags::HARDLINK_DUP),
        is_access_denied: flags.contains(st_core::NodeFlags::ACCESS_DENIED),
        is_cloud_placeholder: flags.contains(st_core::NodeFlags::CLOUD_PLACEHOLDER),
        size_logical,
        size_alloc,
        file_count: tree.file_count(id),
        mtime: tree.mtime(id),
        percent_of_parent: if parent_size == 0 {
            0.0
        } else {
            this_size as f64 * 100.0 / parent_size as f64
        },
    }
}

#[tauri::command]
pub fn list_children(
    state: State<'_, AppState>,
    node_id: u32,
    sort_by: String,
    sort_dir: String,
    use_alloc: bool,
    offset: usize,
    limit: usize,
) -> Result<Vec<RowDto>, String> {
    with_tree(&state, |tree, _root| {
        let parent_size = if use_alloc {
            tree.subtree_alloc(node_id)
        } else {
            tree.subtree_logical(node_id)
        };
        // Anything deleted since the scan is gone from the listing; its
        // bytes have already come off the ancestors' rollups.
        let mut children = tree.live_children(node_id);
        match sort_by.as_str() {
            "name" => children.sort_unstable_by(|&a, &b| tree.name(a).cmp(tree.name(b))),
            _ => children.sort_unstable_by_key(|&id| {
                std::cmp::Reverse(if use_alloc {
                    tree.subtree_alloc(id)
                } else {
                    tree.subtree_logical(id)
                })
            }),
        }
        if sort_dir == "asc" {
            children.reverse();
        }
        children
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|id| row_dto(tree, id, parent_size, use_alloc))
            .collect()
    })
}

#[tauri::command]
pub fn node_info(state: State<'_, AppState>, node_id: u32) -> Result<NodeInfoDto, String> {
    with_tree(&state, |tree, _root| NodeInfoDto {
        id: node_id,
        name: tree.name(node_id).to_string(),
        path: tree.path(node_id, path_sep()),
        parent_id: tree.parent_of(node_id),
        is_dir: tree.is_dir(node_id),
        size_logical: tree.subtree_logical(node_id),
        size_alloc: tree.subtree_alloc(node_id),
        file_count: tree.file_count(node_id),
        mtime: tree.mtime(node_id),
    })
}

#[tauri::command]
pub fn search(
    state: State<'_, AppState>,
    node_id: u32,
    query: String,
) -> Result<Vec<SearchHitDto>, String> {
    with_tree(&state, |tree, _root| {
        core_search::search(tree, node_id, &query)
            .into_iter()
            .filter(|&id| !tree.is_deleted(id))
            .take(500) // enough for a search result list; a full-drive glob easily returns thousands
            .map(|id| SearchHitDto {
                id,
                name: tree.name(id).to_string(),
                path: tree.path(id, path_sep()),
                is_dir: tree.is_dir(id),
                size_logical: tree.subtree_logical(id),
                size_alloc: tree.subtree_alloc(id),
            })
            .collect()
    })
}

/// Resolve a node to a real, absolute filesystem path.
///
/// Every action below goes through this rather than accepting a path from
/// the webview: the frontend knows node ids, and a path it constructed
/// itself would be a path this process never validated.
fn resolve(state: &State<'_, AppState>, node_id: NodeId) -> Result<std::path::PathBuf, String> {
    let guard = state.scan.lock().unwrap();
    let scan = guard.as_ref().ok_or("no scan loaded")?;
    let path = scan
        .absolute_path(node_id)
        .ok_or("that item is not part of the current scan")?;
    // The id crossed an IPC boundary, so treat it as untrusted: a path
    // that resolved outside the scanned root means something is wrong,
    // and this is the last point before a real file operation.
    if !scan.is_within_root(&path) {
        return Err("refusing to act on a path outside the scanned folder".into());
    }
    Ok(path)
}

/// The absolute path of a node, for "Copy path".
#[tauri::command]
pub fn node_path(state: State<'_, AppState>, node_id: NodeId) -> Result<String, String> {
    Ok(resolve(&state, node_id)?.to_string_lossy().into_owned())
}

/// Show the item in the platform's file manager, selected if possible.
#[tauri::command]
pub fn reveal_in_file_manager(state: State<'_, AppState>, node_id: NodeId) -> Result<(), String> {
    let path = resolve(&state, node_id)?;
    reveal(&path).map_err(|e| e.to_string())
}

/// Open the item with whatever the OS considers its default handler.
#[tauri::command]
pub fn open_path(state: State<'_, AppState>, node_id: NodeId) -> Result<(), String> {
    let path = resolve(&state, node_id)?;
    open_with_default_app(&path).map_err(|e| e.to_string())
}

/// Send the item to the Recycle Bin (Trash), then subtract it from the
/// in-memory tree so the sizes on screen stop counting what is gone.
///
/// Recoverable deletion only — there is no permanent-delete path here on
/// purpose. Confirmation is the caller's job and happens before this runs.
/// Returns the reclaimed on-disk bytes so the UI can say what it freed.
#[tauri::command]
pub fn delete_to_trash(state: State<'_, AppState>, node_id: NodeId) -> Result<u64, String> {
    let path = resolve(&state, node_id)?;
    trash::delete(&path)
        .map_err(|e| format!("could not move {} to the Recycle Bin: {e}", path.display()))?;

    let mut guard = state.scan.lock().unwrap();
    let scan = guard.as_mut().ok_or("no scan loaded")?;
    Ok(scan.tree.remove_subtree(node_id).alloc)
}

#[cfg(windows)]
fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    // `/select,` takes the item's own path and opens its parent with the
    // item highlighted. It must be one argument with no space after the
    // comma, which is why this isn't two `arg` calls.
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    // No portable "select this file" on Linux desktops, so open the
    // containing folder — the closest honest equivalent.
    let target = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    std::process::Command::new("xdg-open").arg(target).spawn()?;
    Ok(())
}

#[cfg(windows)]
fn open_with_default_app(path: &std::path::Path) -> std::io::Result<()> {
    // `start` is a cmd builtin, not an executable. The empty string is
    // the window title argument, which `start` would otherwise take a
    // quoted path to be.
    std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_with_default_app(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("open").arg(path).spawn()?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_with_default_app(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(path).spawn()?;
    Ok(())
}

#[tauri::command]
pub fn treemap_layout(
    state: State<'_, AppState>,
    node_id: u32,
    width: f64,
    height: f64,
    use_alloc: bool,
) -> Result<Vec<RectDto>, String> {
    with_tree(&state, |tree, _root| {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
        };
        layout_children_aggregated(tree, node_id, area, use_alloc, MIN_RECT_AREA_PX, MAX_RECTS)
            .into_iter()
            .map(|entry| {
                let (id, name, is_dir, size_alloc, size_logical, aggregated_count) =
                    match entry.cell {
                        TreemapCell::Node(node) => (
                            Some(node),
                            tree.name(node).to_string(),
                            tree.is_dir(node),
                            tree.subtree_alloc(node),
                            tree.subtree_logical(node),
                            0,
                        ),
                        TreemapCell::Aggregate { count, bytes } => (
                            None,
                            format!("{count} smaller items"),
                            true,
                            // The fold happens on whichever measure the view is
                            // showing, so the other one isn't summed — report the
                            // one that was actually used for both.
                            bytes,
                            bytes,
                            count,
                        ),
                    };
                RectDto {
                    id,
                    name,
                    is_dir,
                    size_alloc,
                    size_logical,
                    aggregated_count,
                    x: entry.rect.x,
                    y: entry.rect.y,
                    w: entry.rect.w,
                    h: entry.rect.h,
                }
            })
            .collect()
    })
}

#[tauri::command]
pub fn export_markdown_text(
    state: State<'_, AppState>,
    node_id: u32,
    options: ExportOptionsDto,
) -> Result<String, String> {
    let guard = state.scan.lock().unwrap();
    let scan = guard.as_ref().ok_or("no scan loaded")?;
    let meta = ScanMeta {
        scanned_at: &scan.scanned_at,
        engine: &scan.engine,
        duration: &format!("{:.2}s", scan.duration.as_secs_f64()),
        volume: scan.volume.as_ref(),
    };
    Ok(export_markdown(
        &scan.tree,
        node_id,
        path_sep(),
        &meta,
        &options.into(),
    ))
}

#[tauri::command]
pub async fn save_text_file(
    app: tauri::AppHandle,
    content: String,
    suggested_name: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_file_name(&suggested_name)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let chosen = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| format!("save dialog task panicked: {e}"))?;

    match chosen {
        Some(path) => {
            let path_str = path.to_string();
            let fs_path = path.into_path().map_err(|e| e.to_string())?;
            std::fs::write(&fs_path, content).map_err(|e| e.to_string())?;
            Ok(Some(path_str))
        }
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------
// Settings and the AI assistant
// ---------------------------------------------------------------------

/// Current settings, minus the key itself — the UI never reads it back.
#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> crate::settings::SettingsDto {
    let settings = crate::settings::load(&app);
    let path = crate::settings::config_path_string(&app);
    crate::settings::SettingsDto::from(&settings, path)
}

/// Save settings. `api_key` of `None` leaves the stored key untouched, so
/// changing the model doesn't require re-entering it.
#[tauri::command]
pub fn set_settings(
    app: tauri::AppHandle,
    api_key: Option<String>,
    model: String,
    model_supports_tools: bool,
) -> Result<crate::settings::SettingsDto, String> {
    let mut settings = crate::settings::load(&app);
    if let Some(key) = api_key {
        settings.openrouter_api_key = key.trim().to_string();
    }
    settings.model = model.trim().to_string();
    settings.model_supports_tools = model_supports_tools;
    crate::settings::store(&app, &settings)?;
    let path = crate::settings::config_path_string(&app);
    Ok(crate::settings::SettingsDto::from(&settings, path))
}

/// The live OpenRouter catalogue, free and tool-capable models first.
#[tauri::command]
pub async fn list_models() -> Result<Vec<crate::ai::openrouter::ModelDto>, String> {
    crate::ai::openrouter::fetch_models().await
}

/// Ask the assistant. Streams `ai_delta`, `ai_tool_call`, `ai_done` and
/// `ai_error` events rather than returning the answer, so the panel can
/// render tokens as they arrive.
#[tauri::command]
pub async fn ai_ask(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    question: String,
    history: Vec<crate::ai::HistoryTurn>,
) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    *state.ai_cancel.lock().unwrap() = Some(cancel.clone());

    let result = crate::ai::run_turn(app.clone(), question, history, cancel).await;
    *state.ai_cancel.lock().unwrap() = None;

    if let Err(message) = &result {
        // Emitted as well as returned: the panel renders errors inline in
        // the transcript, where the failed question still is.
        let _ = app.emit(
            "ai_error",
            crate::ai::ErrorEvent {
                message: message.clone(),
            },
        );
    }
    result
}

/// Stop the in-flight answer. The loop checks between chunks and tools.
#[tauri::command]
pub fn ai_cancel(state: State<'_, AppState>) {
    if let Some(flag) = state.ai_cancel.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}
