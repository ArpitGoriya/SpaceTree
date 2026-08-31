//! Golden-file tests for `st_core::export`. Each test builds a small,
//! fully deterministic synthetic tree (no real filesystem involved, so
//! these run identically on any host/CI) and asserts the rendered
//! Markdown byte-for-byte against a committed file under `tests/golden/`.
//!
//! To intentionally update a golden file after a deliberate export
//! format change: run with `UPDATE_GOLDEN=1 cargo test -p st-core`, then
//! diff the rewritten file before committing it.

use st_core::export::{export_markdown, ExportOptions, ScanMeta, SortBy};
use st_core::{NodeFlags, RawNode, Tree, TreeBuilder, VolumeInfo, ROOT};

fn dir(builder: &mut TreeBuilder, parent: u32, name: &str) -> u32 {
    builder.push(RawNode {
        parent,
        name: name.to_string(),
        size_logical: 0,
        size_alloc: 0,
        mtime: 0,
        flags: NodeFlags::DIR,
    })
}

fn file(builder: &mut TreeBuilder, parent: u32, name: &str, size: u64) -> u32 {
    builder.push(RawNode {
        parent,
        name: name.to_string(),
        size_logical: size,
        size_alloc: size.div_ceil(4096) * 4096,
        mtime: 0,
        flags: NodeFlags::empty(),
    })
}

/// Mirrors the shape (not the exact numbers) of the sample in
/// docs/PLAN.md: a drive root with a deep Users/<name>/... branch plus
/// two shallow siblings, and one folder (Videos) with more children than
/// the default top-N so the "more items" collapsing path is exercised.
fn sample_tree() -> (Tree, u32) {
    let mut b = TreeBuilder::new();
    let c = dir(&mut b, ROOT, "C:");

    let users = dir(&mut b, c, "Users");
    let arpit = dir(&mut b, users, "arpit");

    let appdata = dir(&mut b, arpit, "AppData");
    let local = dir(&mut b, appdata, "Local");
    file(&mut b, local, "cache.db", 900_000);
    let roaming = dir(&mut b, appdata, "Roaming");
    file(&mut b, roaming, "settings.json", 2_000);

    let videos = dir(&mut b, arpit, "Videos");
    file(&mut b, videos, "raw-capture-01.mp4", 5_000_000);
    for i in 0..4 {
        file(&mut b, videos, &format!("clip-{i:02}.mp4"), 100_000);
    }

    let downloads = dir(&mut b, arpit, "Downloads");
    file(&mut b, downloads, "installer.exe", 50_000);
    file(&mut b, downloads, "readme", 100); // no extension

    let windows = dir(&mut b, c, "Windows");
    file(&mut b, windows, "explorer.exe", 300_000);

    let program_files = dir(&mut b, c, "Program Files");
    file(&mut b, program_files, "app.dll", 80_000);

    (b.finalize(), c)
}

fn assert_matches_golden(name: &str, actual: &str) {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&path, actual).expect("write golden file");
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading golden file {path}: {e}"));
    assert_eq!(
        actual, expected,
        "\n--- output no longer matches {name} ---\n\
         if this change is intentional, re-run with UPDATE_GOLDEN=1 and review the diff\n"
    );
}

#[test]
fn full_drive_export_matches_golden() {
    let (tree, root) = sample_tree();
    let volume = VolumeInfo {
        label: "Windows-SSD".into(),
        filesystem: "NTFS".into(),
        total_bytes: 1_000_000_000,
        free_bytes: 200_000_000,
        cluster_bytes: 4096,
    };
    let meta = ScanMeta {
        scanned_at: "2026-08-31 14:22:10 +05:30",
        engine: "Parallel walker",
        duration: "0.42 s",
        volume: Some(&volume),
    };
    let opts = ExportOptions {
        max_depth: Some(4),
        min_size: 0,
        include_files: true,
        top_n: Some(2),
        sort_by: SortBy::Size,
        use_alloc: true,
        largest_folders: 5,
        largest_files: 5,
        by_type_limit: 10,
    };

    let out = export_markdown(&tree, root, "\\", &meta, &opts);
    assert_matches_golden("full_drive.md", &out);
}

#[test]
fn folder_only_export_omits_files_and_volume_rows() {
    let (tree, root) = sample_tree();
    let meta = ScanMeta {
        scanned_at: "2026-08-31 14:22:10 +05:30",
        engine: "Parallel walker",
        duration: "0.42 s",
        volume: None,
    };
    let opts = ExportOptions {
        max_depth: None,
        min_size: 0,
        include_files: false,
        top_n: None,
        sort_by: SortBy::Name,
        use_alloc: false,
        largest_folders: 3,
        largest_files: 3,
        by_type_limit: 5,
    };

    let out = export_markdown(&tree, root, "\\", &meta, &opts);
    assert_matches_golden("folder_only.md", &out);

    assert!(
        !out.contains("**Capacity**"),
        "no volume info was given, so no capacity row"
    );
    assert!(
        !out.contains(".mp4"),
        "include_files=false must drop every file row"
    );
}

#[test]
fn min_size_filter_drops_small_entries_from_the_tree_section() {
    let (tree, root) = sample_tree();
    let meta = ScanMeta {
        scanned_at: "2026-08-31 14:22:10 +05:30",
        engine: "Parallel walker",
        duration: "0.42 s",
        volume: None,
    };
    let opts = ExportOptions {
        min_size: 1_000_000,
        max_depth: None,
        top_n: None,
        // Isolate the tree section: the "largest files" ranking is an
        // independent always-top-N view and deliberately ignores
        // min_size, so it must be off for this assertion to be valid.
        largest_folders: 0,
        largest_files: 0,
        ..ExportOptions::default()
    };

    let out = export_markdown(&tree, root, "\\", &meta, &opts);
    // Only raw-capture-01.mp4 (5,000,000 B) clears a 1 MB floor among files.
    assert!(out.contains("raw-capture-01.mp4"));
    assert!(!out.contains("clip-00.mp4"));
    assert!(!out.contains("settings.json"));
}

#[test]
fn by_type_output_is_stable_across_repeated_runs() {
    // extension_breakdown() is backed by a HashMap with randomized
    // iteration order; two extensions tied on size must still render in
    // the same order every run, or exports of an unchanged drive would
    // spuriously diff against each other.
    let (tree, root) = sample_tree();
    let meta = ScanMeta {
        scanned_at: "2026-08-31 14:22:10 +05:30",
        engine: "Parallel walker",
        duration: "0.42 s",
        volume: None,
    };
    let opts = ExportOptions::default();

    let first = export_markdown(&tree, root, "\\", &meta, &opts);
    for _ in 0..20 {
        let (tree, root) = sample_tree();
        let again = export_markdown(&tree, root, "\\", &meta, &opts);
        assert_eq!(
            first, again,
            "export output must not depend on HashMap iteration order"
        );
    }
}
