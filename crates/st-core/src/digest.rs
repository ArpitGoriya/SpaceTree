//! Compact, bounded summaries of a scan, for feeding a language model.
//!
//! The problem this solves: a real scan is millions of nodes. Serialising
//! even a fraction of it into a prompt is both ruinously expensive and
//! useless — the model drowns. So the model never sees the tree. It sees
//! a small digest up front, and calls the functions here to look at
//! specific folders, the way a person would click into them.
//!
//! Every function in this module therefore guarantees two things:
//!
//! 1. **A row cap.** Callers pass a limit; nothing returns unbounded rows.
//! 2. **Honest truncation.** When rows are dropped, the output says so on
//!    its own line. Silently returning the first 20 of 4000 folders would
//!    let the model conclude a drive is small when it isn't.
//!
//! Output is compact text rather than JSON. JSON of this shape costs
//! roughly twice the tokens in punctuation and key repetition, and models
//! read an aligned table at least as well.

use std::collections::HashMap;

use crate::fmt;
use crate::tree::{NodeId, Tree};
use crate::VolumeInfo;

/// Hard ceiling on any single tool result. A model that asks for 500 rows
/// of a folder with 50,000 children should still get a usable answer
/// rather than a wall that blows the context window.
pub const MAX_RESULT_CHARS: usize = 6000;

/// One row of a folder listing.
struct Row {
    name: String,
    size: u64,
    share: f64,
    items: u32,
    is_dir: bool,
}

fn render_rows(rows: &[Row], total_available: usize, header: &str) -> String {
    let mut out = String::with_capacity(header.len() + rows.len() * 48);
    out.push_str(header);
    out.push('\n');
    for row in rows {
        // Trailing slash is the cheapest possible dir/file marker, and
        // one the model already understands from any filesystem listing.
        let name = if row.is_dir {
            format!("{}/", row.name)
        } else {
            row.name.clone()
        };
        let items = if row.is_dir {
            format!("  {} items", fmt::count(row.items as u64))
        } else {
            String::new()
        };
        out.push_str(&format!(
            "{:<40}  {:>10}  {:>6.1}%{}\n",
            truncate_name(&name, 40),
            fmt::bytes(row.size),
            row.share,
            items
        ));
    }
    let shown = rows.len();
    if total_available > shown {
        out.push_str(&format!(
            "... {} more not shown\n",
            fmt::count((total_available - shown) as u64)
        ));
    }
    clamp(out)
}

fn truncate_name(name: &str, width: usize) -> String {
    if name.chars().count() <= width {
        return name.to_string();
    }
    let keep: String = name.chars().take(width.saturating_sub(1)).collect();
    format!("{keep}…")
}

/// Last-resort guard so one pathological result can't blow the budget.
fn clamp(mut s: String) -> String {
    if s.len() <= MAX_RESULT_CHARS {
        return s;
    }
    // Cut on a line boundary so the model never sees half a row.
    let cut = s[..MAX_RESULT_CHARS]
        .rfind('\n')
        .unwrap_or(MAX_RESULT_CHARS);
    s.truncate(cut);
    s.push_str("\n... output truncated to fit\n");
    s
}

fn size_of(tree: &Tree, id: NodeId, use_alloc: bool) -> u64 {
    if use_alloc {
        tree.subtree_alloc(id)
    } else {
        tree.subtree_logical(id)
    }
}

fn share(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

/// The one-time orientation block: what drive this is, how full, and what
/// the biggest things on it are. Goes in the system prompt so the model
/// can answer "what's taking up space" without any tool call at all.
pub fn scan_digest(
    tree: &Tree,
    root: NodeId,
    volume: Option<&VolumeInfo>,
    use_alloc: bool,
    top_folders: usize,
    top_types: usize,
) -> String {
    let mut out = String::new();
    let total = size_of(tree, root, use_alloc);

    out.push_str("SCAN SUMMARY\n");
    if let Some(v) = volume {
        out.push_str(&format!(
            "Drive: {} ({})  capacity {}  used {} ({})  free {}\n",
            v.label,
            v.filesystem,
            fmt::bytes(v.total_bytes),
            fmt::bytes(v.used_bytes()),
            fmt::percent(v.used_bytes(), v.total_bytes),
            fmt::bytes(v.free_bytes),
        ));
    }
    out.push_str(&format!(
        "Scanned root: {}  indexed {}  {} files in {} folders\n",
        tree.name(root),
        fmt::bytes(total),
        fmt::count(tree.file_count(root) as u64),
        fmt::count(tree.descendants(root).filter(|&i| tree.is_dir(i)).count() as u64),
    ));

    out.push_str("\nLARGEST ITEMS AT THE ROOT\n");
    out.push_str(&folder_listing(tree, root, use_alloc, top_folders));

    out.push_str("\nLARGEST FILE TYPES\n");
    out.push_str(&type_breakdown(tree, root, use_alloc, top_types));

    out
}

/// What is directly inside `node`, largest first.
pub fn folder_listing(tree: &Tree, node: NodeId, use_alloc: bool, limit: usize) -> String {
    let mut kids: Vec<NodeId> = tree.live_children(node);
    if kids.is_empty() {
        return "(empty)\n".to_string();
    }
    kids.sort_unstable_by_key(|&id| std::cmp::Reverse(size_of(tree, id, use_alloc)));
    let total = size_of(tree, node, use_alloc);
    let available = kids.len();

    let rows: Vec<Row> = kids
        .iter()
        .take(limit)
        .map(|&id| Row {
            name: tree.name(id).to_string(),
            size: size_of(tree, id, use_alloc),
            share: share(size_of(tree, id, use_alloc), total),
            items: tree.file_count(id),
            is_dir: tree.is_dir(id),
        })
        .collect();

    render_rows(
        &rows,
        available,
        "NAME                                          SIZE   SHARE",
    )
}

/// The biggest individual files anywhere under `node`.
///
/// A folder listing can't answer "what single file is eating 40 GB" when
/// that file is six levels down, which is exactly the question people ask.
pub fn largest_files(
    tree: &Tree,
    node: NodeId,
    use_alloc: bool,
    min_bytes: u64,
    limit: usize,
) -> String {
    let mut files: Vec<(NodeId, u64)> = tree
        .descendants(node)
        .filter(|&id| !tree.is_dir(id) && !tree.is_deleted(id))
        .map(|id| (id, size_of(tree, id, use_alloc)))
        .filter(|&(_, size)| size >= min_bytes)
        .collect();

    if files.is_empty() {
        return format!(
            "No files at or above {} under this folder.\n",
            fmt::bytes(min_bytes)
        );
    }

    files.sort_unstable_by_key(|&(_, size)| std::cmp::Reverse(size));
    let available = files.len();
    let total = size_of(tree, node, use_alloc);

    let mut out = String::from("PATH (relative to the scan root)                SIZE   SHARE\n");
    for &(id, size) in files.iter().take(limit) {
        out.push_str(&format!(
            "{:<44}  {:>10}  {:>6.1}%\n",
            truncate_name(&tree.path(id, "/"), 44),
            fmt::bytes(size),
            share(size, total),
        ));
    }
    if available > limit {
        out.push_str(&format!(
            "... {} more not shown\n",
            fmt::count((available - limit) as u64)
        ));
    }
    clamp(out)
}

/// Which file extensions account for the space under `node`.
pub fn type_breakdown(tree: &Tree, node: NodeId, use_alloc: bool, limit: usize) -> String {
    let map: HashMap<String, crate::ExtStat> = tree.extension_breakdown(node);
    if map.is_empty() {
        return "(no files)\n".to_string();
    }
    let mut rows: Vec<(String, u64, u32)> = map
        .into_iter()
        .map(|(ext, stat)| {
            let size = if use_alloc { stat.alloc } else { stat.logical };
            (ext, size, stat.files)
        })
        .collect();
    // Secondary key on the name keeps this deterministic when two
    // extensions tie, which HashMap order otherwise would not.
    rows.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let available = rows.len();
    let total: u64 = rows.iter().map(|r| r.1).sum();

    let mut out = String::from("TYPE               SIZE   SHARE     FILES\n");
    for (ext, size, files) in rows.iter().take(limit) {
        let label = if ext.is_empty() {
            "(no extension)".to_string()
        } else {
            format!(".{ext}")
        };
        out.push_str(&format!(
            "{:<14}  {:>10}  {:>6.1}%  {:>8}\n",
            truncate_name(&label, 14),
            fmt::bytes(*size),
            share(*size, total),
            fmt::count(*files as u64),
        ));
    }
    if available > limit {
        out.push_str(&format!(
            "... {} more types not shown\n",
            fmt::count((available - limit) as u64)
        ));
    }
    clamp(out)
}

/// Find nodes whose name contains `needle` (case-insensitive), largest
/// first. Answers "how much is node_modules costing me across the disk".
pub fn find_by_name(
    tree: &Tree,
    root: NodeId,
    needle: &str,
    use_alloc: bool,
    limit: usize,
) -> String {
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return "(empty search term)\n".to_string();
    }
    let mut hits: Vec<(NodeId, u64)> = tree
        .descendants(root)
        .filter(|&id| !tree.is_deleted(id) && tree.name(id).to_lowercase().contains(&needle))
        .map(|id| (id, size_of(tree, id, use_alloc)))
        .collect();

    if hits.is_empty() {
        return format!("Nothing under this folder matches \"{needle}\".\n");
    }
    hits.sort_unstable_by_key(|&(_, size)| std::cmp::Reverse(size));
    let available = hits.len();
    let combined: u64 = hits.iter().map(|h| h.1).sum();

    let mut out = format!(
        "{} matches, {} combined\nPATH                                            SIZE\n",
        fmt::count(available as u64),
        fmt::bytes(combined),
    );
    for &(id, size) in hits.iter().take(limit) {
        out.push_str(&format!(
            "{:<44}  {:>10}\n",
            truncate_name(&tree.path(id, "/"), 44),
            fmt::bytes(size),
        ));
    }
    if available > limit {
        out.push_str(&format!(
            "... {} more not shown\n",
            fmt::count((available - limit) as u64)
        ));
    }
    clamp(out)
}

/// Detail on one node, for when the model wants to check a specific thing
/// before recommending anything about it.
pub fn node_detail(tree: &Tree, id: NodeId, use_alloc: bool) -> String {
    let kind = if tree.is_dir(id) { "folder" } else { "file" };
    let mut out = format!(
        "{}: {}\nkind: {}\nsize: {}\n",
        tree.name(id),
        tree.path(id, "/"),
        kind,
        fmt::bytes(size_of(tree, id, use_alloc)),
    );
    if tree.is_dir(id) {
        out.push_str(&format!(
            "contains: {} files\ndirect children: {}\n",
            fmt::count(tree.file_count(id) as u64),
            fmt::count(tree.live_children(id).len() as u64),
        ));
    }
    if let Some(note) = classify(tree.name(id), tree.is_dir(id)) {
        out.push_str(&format!(
            "category: {}\ncaution: {}\n",
            note.category, note.caution
        ));
    }
    out
}

// ---------------------------------------------------------------------
// Reclaimable-space classification
// ---------------------------------------------------------------------

/// What a well-known folder or file name means, and what the risk is.
///
/// This exists because the model cannot be trusted to know it. Ask a
/// small model "can I delete C:\Windows\Installer" and it will often say
/// yes — it is a cache-sounding name full of stale MSI files, and
/// deleting it silently breaks every future repair and uninstall. The
/// caution has to be a *fact in the tool result*, not a hope about what
/// the model learned.
pub struct Verdict {
    pub category: &'static str,
    /// How safe this is to remove, in plain words the model will repeat.
    pub caution: &'static str,
    /// Whether this is generally safe to clear.
    pub generally_safe: bool,
}

/// Classify a node by name. `None` means "nothing special known".
pub fn classify(name: &str, is_dir: bool) -> Option<Verdict> {
    let lower = name.to_lowercase();

    // Files first — a couple of Windows system files get asked about a lot.
    if !is_dir {
        return match lower.as_str() {
            "hiberfil.sys" => Some(Verdict {
                category: "Windows hibernation file",
                caution: "Do not delete. Sized by Windows; disable hibernation with `powercfg /h off` if you want the space back.",
                generally_safe: false,
            }),
            "pagefile.sys" | "swapfile.sys" => Some(Verdict {
                category: "Windows virtual memory",
                caution: "Do not delete. Managed by Windows; change it in System > Advanced > Virtual memory instead.",
                generally_safe: false,
            }),
            _ => None,
        };
    }

    let verdict = match lower.as_str() {
        "node_modules" => Verdict {
            category: "JavaScript dependencies",
            caution: "Safe to delete; `npm install` rebuilds it. Only matters if the project must work offline.",
            generally_safe: true,
        },
        "__pycache__" | ".pytest_cache" | ".mypy_cache" | ".ruff_cache" => Verdict {
            category: "Python cache",
            caution: "Safe to delete; regenerated automatically.",
            generally_safe: true,
        },
        "target" => Verdict {
            category: "Rust build output",
            caution: "Safe to delete if this is a Rust project; `cargo build` rebuilds it, though the next build is slow.",
            generally_safe: true,
        },
        "build" | "dist" | "out" | "obj" | "bin" => Verdict {
            category: "Build output",
            caution: "Usually safe if this is a project folder and you can rebuild. Check it isn't an application's install directory first.",
            generally_safe: true,
        },
        ".gradle" | ".m2" | ".ivy2" | ".nuget" | ".cargo" => Verdict {
            category: "Package manager cache",
            caution: "Safe to delete; re-downloaded on next build. Costs bandwidth, not data.",
            generally_safe: true,
        },
        "cache" | "caches" | "cachestorage" | "code cache" | "gpucache" => Verdict {
            category: "Application cache",
            caution: "Generally safe; the app rebuilds it. Close the app first.",
            generally_safe: true,
        },
        "temp" | "tmp" => Verdict {
            category: "Temporary files",
            caution: "Generally safe when nothing is mid-install. Some entries will be locked and skipped.",
            generally_safe: true,
        },
        "$recycle.bin" => Verdict {
            category: "Recycle Bin",
            caution: "Safe, but this is deleted files you can still restore. Empty it from Explorer rather than deleting the folder.",
            generally_safe: true,
        },
        "downloads" => Verdict {
            category: "Downloads",
            caution: "Often reclaimable, but it is your data and nothing regenerates it. Review before removing anything.",
            generally_safe: false,
        },
        "softwaredistribution" => Verdict {
            category: "Windows Update cache",
            caution: "Only the Download subfolder is reclaimable, and only via Disk Cleanup or by stopping the Windows Update service first.",
            generally_safe: false,
        },
        "winsxs" => Verdict {
            category: "Windows component store",
            caution: "Never delete. Much of its apparent size is hard links counted twice. Use `DISM /Online /Cleanup-Image /StartComponentCleanup`.",
            generally_safe: false,
        },
        "installer" => Verdict {
            category: "Windows Installer cache",
            caution: "Never delete manually. Removing it breaks repair, update and uninstall for installed software.",
            generally_safe: false,
        },
        "system volume information" => Verdict {
            category: "Restore points and shadow copies",
            caution: "Never delete. Reduce it through System Protection settings instead.",
            generally_safe: false,
        },
        "appdata" => Verdict {
            category: "Application data",
            caution: "Do not delete wholesale — it holds live settings and profiles. Look inside for caches instead.",
            generally_safe: false,
        },
        _ => return None,
    };
    Some(verdict)
}

/// Walk the scan for anything the classifier recognises, largest first.
///
/// This is the tool that makes "what can I delete" answerable with
/// specifics rather than generalities. It reports unsafe categories too,
/// clearly marked — the model needs to be able to say "that big folder is
/// WinSxS, leave it alone", which is often the most useful answer.
pub fn reclaimable(tree: &Tree, root: NodeId, use_alloc: bool, limit: usize) -> String {
    let mut hits: Vec<(NodeId, u64, Verdict)> = Vec::new();
    for id in tree.descendants(root) {
        if tree.is_deleted(id) {
            continue;
        }
        if let Some(verdict) = classify(tree.name(id), tree.is_dir(id)) {
            hits.push((id, size_of(tree, id, use_alloc), verdict));
        }
    }

    if hits.is_empty() {
        return "No well-known cache, build-output or system folders found in this scan.\n"
            .to_string();
    }
    hits.sort_unstable_by_key(|h| std::cmp::Reverse(h.1));

    // A nested match (a `Cache` inside a `node_modules`) would double
    // count, so keep only the outermost of any nested pair.
    let kept: Vec<&(NodeId, u64, Verdict)> = hits
        .iter()
        .filter(|(id, _, _)| {
            let mut cursor = tree.parent_of(*id);
            while let Some(p) = cursor {
                if hits.iter().any(|(other, _, _)| other == &p) {
                    return false;
                }
                cursor = tree.parent_of(p);
            }
            true
        })
        .collect();

    let safe_total: u64 = kept
        .iter()
        .filter(|(_, _, v)| v.generally_safe)
        .map(|(_, size, _)| size)
        .sum();

    let mut out = format!(
        "Recognised {} candidates. Generally-safe ones total {}.\n\n",
        fmt::count(kept.len() as u64),
        fmt::bytes(safe_total),
    );
    for (id, size, verdict) in kept.iter().take(limit) {
        out.push_str(&format!(
            "{}  [{}]\n  path: {}\n  {} — {}\n",
            fmt::bytes(*size),
            if verdict.generally_safe {
                "generally safe"
            } else {
                "DO NOT DELETE without reading the note"
            },
            tree.path(*id, "/"),
            verdict.category,
            verdict.caution,
        ));
    }
    if kept.len() > limit {
        out.push_str(&format!(
            "... {} more not shown\n",
            fmt::count((kept.len() - limit) as u64)
        ));
    }
    clamp(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{RawNode, TreeBuilder, ROOT};
    use crate::NodeFlags;

    fn dir(parent: NodeId, name: &str) -> RawNode {
        RawNode {
            parent,
            name: name.to_string(),
            size_logical: 0,
            size_alloc: 0,
            mtime: 0,
            flags: NodeFlags::DIR,
        }
    }

    fn file(parent: NodeId, name: &str, size: u64) -> RawNode {
        RawNode {
            parent,
            name: name.to_string(),
            size_logical: size,
            size_alloc: size,
            mtime: 0,
            flags: NodeFlags::empty(),
        }
    }

    fn sample() -> (Tree, NodeId) {
        let mut b = TreeBuilder::new();
        let root = b.push(dir(ROOT, "C:"));
        let users = b.push(dir(root, "Users"));
        let proj = b.push(dir(users, "project"));
        b.push(dir(proj, "node_modules"));
        b.push(file(proj, "main.rs", 5_000));
        b.push(file(users, "movie.mkv", 8_000_000_000));
        let win = b.push(dir(root, "Windows"));
        b.push(dir(win, "WinSxS"));
        b.push(file(win, "hiberfil.sys", 3_000_000_000));
        (b.finalize(), root)
    }

    #[test]
    fn a_listing_is_capped_and_says_what_it_dropped() {
        let mut b = TreeBuilder::new();
        let root = b.push(dir(ROOT, "root"));
        for i in 0..500 {
            b.push(file(root, &format!("f{i}.bin"), 1000 - i as u64));
        }
        let tree = b.finalize();

        let out = folder_listing(&tree, root, true, 10);
        assert_eq!(
            out.lines().filter(|l| l.starts_with('f')).count(),
            10,
            "must honour the row cap"
        );
        assert!(
            out.contains("490 more not shown"),
            "truncation must be stated, not silent:\n{out}"
        );
    }

    #[test]
    fn every_result_stays_within_the_character_budget() {
        let mut b = TreeBuilder::new();
        let root = b.push(dir(ROOT, "root"));
        for i in 0..5000 {
            b.push(file(
                root,
                &format!("a-very-long-file-name-number-{i}.bin"),
                1,
            ));
        }
        let tree = b.finalize();

        // Even asking for far more rows than the budget allows.
        let out = folder_listing(&tree, root, true, 5000);
        assert!(
            out.len() <= MAX_RESULT_CHARS + 40,
            "result was {} chars, over the budget",
            out.len()
        );
        assert!(out.contains("truncated to fit"));
        assert!(out.ends_with('\n'), "must not cut mid-row");
    }

    #[test]
    fn largest_files_reaches_deep_into_the_tree() {
        let (tree, root) = sample();
        let out = largest_files(&tree, root, true, 1_000_000, 10);
        assert!(
            out.contains("movie.mkv"),
            "a big file nested two levels down must be found:\n{out}"
        );
        assert!(
            !out.contains("main.rs"),
            "files under the minimum must be excluded:\n{out}"
        );
    }

    #[test]
    fn dangerous_folders_are_flagged_as_such() {
        let winsxs = classify("WinSxS", true).expect("WinSxS must be recognised");
        assert!(!winsxs.generally_safe);
        assert!(winsxs.caution.contains("Never delete"));

        let installer = classify("Installer", true).expect("Installer must be recognised");
        assert!(!installer.generally_safe);

        let hib = classify("hiberfil.sys", false).expect("hiberfil must be recognised");
        assert!(!hib.generally_safe);
        assert!(hib.caution.contains("powercfg"));
    }

    #[test]
    fn safe_caches_are_recognised_and_case_insensitive() {
        for name in [
            "node_modules",
            "NODE_MODULES",
            "Cache",
            "__pycache__",
            ".gradle",
        ] {
            let v = classify(name, true).unwrap_or_else(|| panic!("{name} should be classified"));
            assert!(v.generally_safe, "{name} should read as generally safe");
        }
    }

    #[test]
    fn a_file_named_like_a_cache_folder_is_not_classified_as_one() {
        // "cache" as a *file* is just a file; the folder rules must not
        // leak across, or the model gets told a random file is safe to bin.
        assert!(classify("cache", false).is_none());
        assert!(classify("node_modules", false).is_none());
    }

    #[test]
    fn reclaimable_reports_both_safe_and_unsafe_candidates() {
        let (tree, root) = sample();
        let out = reclaimable(&tree, root, true, 20);
        assert!(out.contains("node_modules"), "{out}");
        assert!(
            out.contains("WinSxS"),
            "unsafe ones must still be listed:\n{out}"
        );
        assert!(
            out.contains("DO NOT DELETE"),
            "unsafe candidates must carry the warning:\n{out}"
        );
    }

    #[test]
    fn nested_candidates_are_not_double_counted() {
        let mut b = TreeBuilder::new();
        let root = b.push(dir(ROOT, "root"));
        let nm = b.push(dir(root, "node_modules"));
        let inner = b.push(dir(nm, "cache"));
        b.push(file(inner, "blob", 1_000_000));
        let tree = b.finalize();

        let out = reclaimable(&tree, root, true, 20);
        assert!(out.contains("node_modules"));
        assert!(
            !out.contains("root/node_modules/cache"),
            "a cache inside node_modules would count its bytes twice:\n{out}"
        );
    }

    #[test]
    fn the_digest_orients_without_any_tool_call() {
        let (tree, root) = sample();
        let out = scan_digest(&tree, root, None, true, 10, 5);
        assert!(out.contains("SCAN SUMMARY"));
        assert!(
            out.contains("Windows"),
            "top-level folders must appear:\n{out}"
        );
        assert!(out.contains("LARGEST FILE TYPES"));
        assert!(
            out.len() < 4000,
            "the digest rides in every request; it was {} chars",
            out.len()
        );
    }

    #[test]
    fn find_by_name_totals_matches_across_the_tree() {
        let (tree, root) = sample();
        let out = find_by_name(&tree, root, "node_modules", true, 10);
        assert!(
            out.contains("1 matches") || out.contains("1 match"),
            "{out}"
        );
    }

    #[test]
    fn empty_and_missing_cases_do_not_panic() {
        let mut b = TreeBuilder::new();
        let root = b.push(dir(ROOT, "empty"));
        let tree = b.finalize();
        assert!(folder_listing(&tree, root, true, 10).contains("empty"));
        assert!(largest_files(&tree, root, true, 1, 10).contains("No files"));
        assert!(type_breakdown(&tree, root, true, 10).contains("no files"));
        assert!(find_by_name(&tree, root, "", true, 10).contains("empty search"));
    }
}
