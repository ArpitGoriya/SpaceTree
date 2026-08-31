//! Renders a [`Tree`] as the Markdown report described in the project
//! plan: a summary table, a box-drawing folder tree with aligned size
//! and percentage columns, and largest-folders / largest-files /
//! by-extension tables.
//!
//! Two things keep this predictable at scan scale (millions of nodes):
//! - The tree section walks recursively bounded by `opts.max_depth` and
//!   real filesystem nesting (which Windows itself caps in practice) —
//!   unlike the untrusted on-disk MFT parser in [`crate::tree`], this
//!   walks an already-materialized, already-validated tree, so plain
//!   recursion is the readable choice here.
//! - The ranking tables never sort the whole tree; they keep a bounded
//!   min-heap while streaming [`Tree::descendants`].

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::tree::{NodeId, Tree};
use crate::{fmt, VolumeInfo};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Size,
    Name,
}

/// Controls what the exported document includes. `Default` matches the
/// clipboard-safe preset from the plan: depth 4, top 20 children per
/// folder, on-disk size.
#[derive(Clone, Debug)]
pub struct ExportOptions {
    /// `None` = no limit (full tree — the "Export to file" path).
    pub max_depth: Option<u32>,
    /// Entries smaller than this are dropped from the tree section.
    pub min_size: u64,
    /// `false` renders folders only, skipping individual files.
    pub include_files: bool,
    /// Cap on children rendered per folder; the remainder collapses
    /// into one `… N more items` row. `None` = no cap.
    pub top_n: Option<u32>,
    pub sort_by: SortBy,
    /// `true` = on-disk (allocated) size, `false` = logical size. Applies
    /// to every number in the document, matching the header toggle.
    pub use_alloc: bool,
    pub largest_folders: usize,
    pub largest_files: usize,
    pub by_type_limit: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            max_depth: Some(4),
            min_size: 0,
            include_files: true,
            top_n: Some(20),
            sort_by: SortBy::Size,
            use_alloc: true,
            largest_folders: 10,
            largest_files: 10,
            by_type_limit: 20,
        }
    }
}

/// Scan metadata shown in the header table. All strings are pre-formatted
/// by the caller (CLI or app) so this module stays free of date/duration
/// formatting policy.
pub struct ScanMeta<'a> {
    pub scanned_at: &'a str,
    pub engine: &'a str,
    pub duration: &'a str,
    pub volume: Option<&'a VolumeInfo>,
}

pub fn export_markdown(
    tree: &Tree,
    root: NodeId,
    path_sep: &str,
    meta: &ScanMeta,
    opts: &ExportOptions,
) -> String {
    let mut out = String::new();
    let root_path = tree.path(root, path_sep);
    let root_size = size_of(tree, root, opts);

    write_title(&mut out, &root_path, meta);
    write_summary_table(&mut out, tree, root, meta, root_size);
    write_tree_section(&mut out, tree, root, root_size, opts);

    if opts.largest_folders > 0 {
        write_ranking_table(
            &mut out,
            "Largest folders",
            &["#", "Path", "Size", "% of total"],
            largest(tree, root, opts, true)
                .into_iter()
                .enumerate()
                .map(|(i, (id, size))| {
                    vec![
                        (i + 1).to_string(),
                        code_span(&tree.path(id, path_sep)),
                        fmt::bytes(size),
                        fmt::percent(size, root_size),
                    ]
                }),
        );
    }

    // A folders-only report (opts.include_files = false) skips every
    // section that only makes sense in terms of individual files.
    if opts.include_files {
        if opts.largest_files > 0 {
            write_ranking_table(
                &mut out,
                "Largest files",
                &["#", "Path", "Size"],
                largest(tree, root, opts, false)
                    .into_iter()
                    .enumerate()
                    .map(|(i, (id, size))| {
                        vec![
                            (i + 1).to_string(),
                            code_span(&tree.path(id, path_sep)),
                            fmt::bytes(size),
                        ]
                    }),
            );
        }
        if opts.by_type_limit > 0 {
            write_by_type_table(&mut out, tree, root, opts, root_size);
        }
    }

    out
}

fn size_of(tree: &Tree, id: NodeId, opts: &ExportOptions) -> u64 {
    if opts.use_alloc {
        tree.subtree_alloc(id)
    } else {
        tree.subtree_logical(id)
    }
}

fn write_title(out: &mut String, root_path: &str, meta: &ScanMeta) {
    match meta.volume {
        Some(v) if !v.label.is_empty() => {
            out.push_str(&format!("# SpaceTree — {root_path} ({})\n\n", v.label));
        }
        _ => out.push_str(&format!("# SpaceTree — {root_path}\n\n")),
    }
}

fn write_summary_table(
    out: &mut String,
    tree: &Tree,
    root: NodeId,
    meta: &ScanMeta,
    root_size: u64,
) {
    out.push_str("| | |\n|---|---|\n");
    out.push_str(&format!("| **Scanned** | {} |\n", meta.scanned_at));
    out.push_str(&format!(
        "| **Engine** | {} · {} |\n",
        meta.engine, meta.duration
    ));

    if let Some(vol) = meta.volume {
        out.push_str(&format!(
            "| **Filesystem** | {} · {} clusters |\n",
            vol.filesystem,
            fmt::bytes(vol.cluster_bytes as u64)
        ));
        out.push_str(&format!(
            "| **Capacity** | {} |\n",
            fmt::bytes(vol.total_bytes)
        ));
        out.push_str(&format!(
            "| **Used** | {} ({}) |\n",
            fmt::bytes(vol.used_bytes()),
            fmt::percent(vol.used_bytes(), vol.total_bytes)
        ));
        out.push_str(&format!(
            "| **Free** | {} ({}) |\n",
            fmt::bytes(vol.free_bytes),
            fmt::percent(vol.free_bytes, vol.total_bytes)
        ));
    }

    out.push_str(&format!(
        "| **Indexed** | {} files · {} folders · {} |\n\n",
        fmt::count(count_files(tree, root) as u64),
        fmt::count(count_dirs(tree, root) as u64),
        fmt::bytes(root_size)
    ));

    if let Some(vol) = meta.volume {
        let used = vol.used_bytes();
        if root_size < used {
            let delta = used - root_size;
            out.push_str(&format!(
                "> Indexed total is {} below drive \"used\" — common causes are files \
                 outside the scanned folder, in-use system files, and other users' data.\n\n",
                fmt::bytes(delta)
            ));
        }
    }
}

fn count_files(tree: &Tree, root: NodeId) -> u32 {
    tree.file_count(root)
}

fn count_dirs(tree: &Tree, root: NodeId) -> u32 {
    tree.descendants(root).filter(|&id| tree.is_dir(id)).count() as u32 - 1 // exclude root itself
}

fn write_tree_section(
    out: &mut String,
    tree: &Tree,
    root: NodeId,
    root_size: u64,
    opts: &ExportOptions,
) {
    let mut rows = Vec::new();
    walk(tree, root, 0, "", true, true, opts, root_size, &mut rows);

    let name_w = rows
        .iter()
        .map(|r| r.label.chars().count())
        .max()
        .unwrap_or(0);
    let size_w = rows
        .iter()
        .map(|r| r.size_str.chars().count())
        .max()
        .unwrap_or(0);
    let pct_w = rows
        .iter()
        .map(|r| r.pct_str.chars().count())
        .max()
        .unwrap_or(0);

    out.push_str("## Folder tree\n\n```text\n");
    for row in &rows {
        let line = format!(
            "{:<name_w$}  {:>size_w$}  {:>pct_w$}  {}",
            row.label,
            row.size_str,
            row.pct_str,
            row.count_str,
            name_w = name_w,
            size_w = size_w,
            pct_w = pct_w,
        );
        // Files carry no count, which would otherwise leave a ragged
        // trailing gap on every such line.
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.push_str("```\n\n");
}

struct Row {
    label: String,
    size_str: String,
    pct_str: String,
    count_str: String,
}

#[allow(clippy::too_many_arguments)]
fn walk(
    tree: &Tree,
    node: NodeId,
    depth: u32,
    prefix: &str,
    is_last: bool,
    is_root: bool,
    opts: &ExportOptions,
    root_size: u64,
    rows: &mut Vec<Row>,
) {
    let size = size_of(tree, node, opts);
    let label = if is_root {
        tree.name(node).to_string()
    } else {
        format!(
            "{prefix}{}{}",
            if is_last { "└── " } else { "├── " },
            tree.name(node)
        )
    };
    let count_str = if tree.is_dir(node) {
        let n = fmt::count(tree.file_count(node) as u64);
        if is_root {
            format!("{n} files")
        } else {
            n
        }
    } else {
        String::new()
    };
    rows.push(Row {
        label,
        size_str: fmt::bytes(size),
        pct_str: fmt::percent(size, root_size),
        count_str,
    });

    if !tree.is_dir(node) {
        return;
    }
    if let Some(max_depth) = opts.max_depth {
        if depth >= max_depth {
            return;
        }
    }

    let child_prefix = if is_root {
        String::new()
    } else {
        format!("{prefix}{}", if is_last { "    " } else { "│   " })
    };

    let mut children: Vec<NodeId> = tree
        .children(node)
        .iter()
        .copied()
        .filter(|&c| opts.include_files || tree.is_dir(c))
        .filter(|&c| size_of(tree, c, opts) >= opts.min_size)
        .collect();
    match opts.sort_by {
        SortBy::Size => {
            children.sort_unstable_by_key(|&c| Reverse(size_of(tree, c, opts)));
        }
        SortBy::Name => children.sort_unstable_by(|&a, &b| tree.name(a).cmp(tree.name(b))),
    }

    let split = match opts.top_n {
        Some(n) if (n as usize) < children.len() => n as usize,
        _ => children.len(),
    };
    let (visible, rest) = children.split_at(split);

    for (i, &child) in visible.iter().enumerate() {
        let last_here = rest.is_empty() && i == visible.len() - 1;
        walk(
            tree,
            child,
            depth + 1,
            &child_prefix,
            last_here,
            false,
            opts,
            root_size,
            rows,
        );
    }

    if !rest.is_empty() {
        let rest_size: u64 = rest.iter().map(|&c| size_of(tree, c, opts)).sum();
        let noun = if rest.len() == 1 { "item" } else { "items" };
        rows.push(Row {
            label: format!("{child_prefix}└── … {} more {noun}", rest.len()),
            size_str: fmt::bytes(rest_size),
            pct_str: fmt::percent(rest_size, root_size),
            count_str: String::new(),
        });
    }
}

/// Top-`N` folders (by subtree size) or files (by own size) under `root`,
/// largest first. `root` itself is excluded from the folder ranking.
/// Streams [`Tree::descendants`] through a bounded min-heap rather than
/// sorting every node, so this stays cheap even at millions of entries.
fn largest(tree: &Tree, root: NodeId, opts: &ExportOptions, folders: bool) -> Vec<(NodeId, u64)> {
    let cap = if folders {
        opts.largest_folders
    } else {
        opts.largest_files
    };
    if cap == 0 {
        return Vec::new();
    }
    let mut heap: BinaryHeap<Reverse<(u64, NodeId)>> = BinaryHeap::with_capacity(cap + 1);
    for id in tree.descendants(root) {
        if id == root {
            continue;
        }
        if tree.is_dir(id) != folders {
            continue;
        }
        let size = size_of(tree, id, opts);
        heap.push(Reverse((size, id)));
        if heap.len() > cap {
            heap.pop();
        }
    }
    let mut items: Vec<(NodeId, u64)> = heap.into_iter().map(|Reverse((s, id))| (id, s)).collect();
    items.sort_unstable_by_key(|&(_, s)| Reverse(s));
    items
}

fn write_ranking_table(
    out: &mut String,
    title: &str,
    headers: &[&str],
    rows: impl Iterator<Item = Vec<String>>,
) {
    out.push_str(&format!("## {title}\n\n"));
    out.push_str(&format!("| {} |\n", headers.join(" | ")));
    // "---|" already supplies each column's own trailing pipe, so the
    // format string adds only the one leading pipe, not a matching one.
    out.push_str(&format!("|{}\n", "---|".repeat(headers.len())));
    let mut any = false;
    for row in rows {
        any = true;
        out.push_str(&format!("| {} |\n", row.join(" | ")));
    }
    if !any {
        out.push_str(&format!("| {} |\n", vec!["—"; headers.len()].join(" | ")));
    }
    out.push('\n');
}

fn write_by_type_table(
    out: &mut String,
    tree: &Tree,
    root: NodeId,
    opts: &ExportOptions,
    root_size: u64,
) {
    let breakdown = tree.extension_breakdown(root);
    let mut entries: Vec<(String, u64, u32)> = breakdown
        .into_iter()
        .map(|(ext, stat)| {
            let size = if opts.use_alloc {
                stat.alloc
            } else {
                stat.logical
            };
            (ext, size, stat.files)
        })
        .collect();
    // HashMap iteration order is randomized per run, so entries tied on
    // size (a real occurrence — see the golden-test failure this fixed)
    // need a deterministic secondary key or output isn't reproducible
    // between scans, breaking the diff-cleanly guarantee the tree
    // section already gets from its own explicit sort.
    entries.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    out.push_str("## By file type\n\n| Extension | Size | % | Files |\n|---|---|---|---|\n");
    if entries.is_empty() {
        out.push_str("| — | — | — | — |\n\n");
        return;
    }

    let (shown, rest) = entries.split_at(entries.len().min(opts.by_type_limit));
    for (ext, size, files) in shown {
        let label = if ext == "(none)" {
            ext.clone()
        } else {
            format!(".{ext}")
        };
        out.push_str(&format!(
            "| {label} | {} | {} | {} |\n",
            fmt::bytes(*size),
            fmt::percent(*size, root_size),
            fmt::count(*files as u64)
        ));
    }
    if !rest.is_empty() {
        let rest_size: u64 = rest.iter().map(|&(_, s, _)| s).sum();
        let rest_files: u32 = rest.iter().map(|&(_, _, f)| f).sum();
        out.push_str(&format!(
            "| *(+{} more types)* | {} | {} | {} |\n",
            rest.len(),
            fmt::bytes(rest_size),
            fmt::percent(rest_size, root_size),
            fmt::count(rest_files as u64)
        ));
    }
    out.push('\n');
}

/// Wraps `s` as a Markdown inline code span, safe against embedded
/// backticks and pipes (the latter can't occur in a Windows path, but
/// portable engines targeting Linux/macOS can produce one).
fn code_span(s: &str) -> String {
    let escaped = s.replace('|', "\\|");
    if escaped.contains('`') {
        format!("``{escaped}``")
    } else {
        format!("`{escaped}`")
    }
}
