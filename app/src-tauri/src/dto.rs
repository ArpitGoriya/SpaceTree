//! Wire types for the frontend. Kept separate from `st_core`'s own
//! types (rather than `#[derive(Serialize)]` on them directly) so the
//! JSON contract can stay stable and friendly — booleans instead of a
//! flags bitmask, pre-formatted strings — independent of internal
//! representation changes on the Rust side.

use serde::{Deserialize, Serialize};

use st_core::export::{ExportOptions, SortBy};
use st_core::VolumeInfo;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VolumeDto {
    pub path: String,
    pub label: String,
    pub filesystem: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
}

impl VolumeDto {
    pub fn new(path: String, label: String, info: &VolumeInfo) -> Self {
        Self {
            path,
            label,
            filesystem: info.filesystem.clone(),
            total_bytes: info.total_bytes,
            used_bytes: info.used_bytes(),
            free_bytes: info.free_bytes,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FastScanStatusDto {
    /// The path is a whole NTFS volume, so the MFT engine applies to it.
    pub available: bool,
    /// This process can already open the raw volume.
    pub elevated: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgressDto {
    pub files_seen: u64,
    /// On-disk bytes of file content found so far — the same measure the
    /// finished header reports, so the running number and the final one
    /// are the same quantity.
    pub bytes_seen: u64,
    pub elapsed_ms: u64,
    /// Which engine is running, so the scanning screen never has to
    /// guess (it used to say "Parallel walker" unconditionally).
    pub engine: String,
    /// `"indexing"` or `"buildingTree"`.
    pub phase: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderDto {
    pub root_id: u32,
    pub root_name: String,
    pub root_path: String,
    pub engine: String,
    pub duration_ms: u64,
    pub scanned_at: String,
    pub denied_count: u64,
    pub volume: Option<VolumeDto>,
    pub indexed_files: u32,
    pub indexed_folders: u32,
    pub indexed_logical: u64,
    pub indexed_alloc: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RowDto {
    pub id: u32,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_hardlink_dup: bool,
    pub is_access_denied: bool,
    pub is_cloud_placeholder: bool,
    pub size_logical: u64,
    pub size_alloc: u64,
    pub file_count: u32,
    pub mtime: i64,
    /// 0..100, relative to the size of the listing's own parent (the
    /// node `list_children` was called on), using whichever size mode
    /// the caller asked for.
    pub percent_of_parent: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfoDto {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub parent_id: Option<u32>,
    pub is_dir: bool,
    pub size_logical: u64,
    pub size_alloc: u64,
    pub file_count: u32,
    pub mtime: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size_logical: u64,
    pub size_alloc: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RectDto {
    /// `None` marks the synthetic rect standing in for the folders too
    /// small to draw individually — it has no node to select or drill
    /// into, which is exactly what a null id tells the frontend.
    pub id: Option<u32>,
    pub name: String,
    pub is_dir: bool,
    pub size_alloc: u64,
    pub size_logical: u64,
    /// How many folders this rect stands for; 0 for a real one.
    pub aggregated_count: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptionsDto {
    pub max_depth: Option<u32>,
    pub min_size: u64,
    pub include_files: bool,
    pub top_n: Option<u32>,
    pub sort_by: String,
    pub use_alloc: bool,
    pub largest_folders: usize,
    pub largest_files: usize,
    pub by_type_limit: usize,
}

impl From<ExportOptionsDto> for ExportOptions {
    fn from(d: ExportOptionsDto) -> Self {
        ExportOptions {
            max_depth: d.max_depth,
            min_size: d.min_size,
            include_files: d.include_files,
            top_n: d.top_n,
            sort_by: if d.sort_by == "name" {
                SortBy::Name
            } else {
                SortBy::Size
            },
            use_alloc: d.use_alloc,
            largest_folders: d.largest_folders,
            largest_files: d.largest_files,
            by_type_limit: d.by_type_limit,
        }
    }
}
