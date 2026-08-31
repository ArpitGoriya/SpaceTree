//! Dev-only harness for exercising the scan engine and exporter without
//! a GUI. Not a shipped product surface — see docs/PLAN.md's Architecture
//! section: the real product is the Tauri app, and this exists purely to
//! prove the underlying crates end to end during development.

use std::env;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use st_core::export::{export_markdown, ExportOptions, ScanMeta};
use st_core::volume;
use st_scan::scan;

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("scan") => cmd_scan(args.get(2).map(String::as_str).unwrap_or(".")),
        _ => {
            eprintln!("usage: st-cli scan <path>");
            std::process::exit(2);
        }
    }
}

fn cmd_scan(path: &str) {
    let root = Path::new(path);
    let cancel = AtomicBool::new(false);
    let start = Instant::now();

    let result = scan(root, &cancel, |p| {
        eprint!(
            "\rscanning… {} files, {} — {:.1}s   ",
            st_core::fmt::count(p.files_seen),
            st_core::fmt::bytes(p.bytes_seen),
            p.elapsed.as_secs_f64()
        );
    })
    .unwrap_or_else(|e| {
        eprintln!("\nscan failed: {e}");
        std::process::exit(1);
    });
    eprintln!();

    let elapsed = start.elapsed();
    let tree = &result.tree;
    let files = tree.file_count(result.root);
    let bytes = tree.subtree_alloc(result.root);

    eprintln!(
        "done: {} files in {} folders, {} on disk, {:.2}s ({} files/s), {} folders not readable",
        st_core::fmt::count(files as u64),
        st_core::fmt::count((tree.node_count() as u64).saturating_sub(files as u64 + 1)),
        st_core::fmt::bytes(bytes),
        elapsed.as_secs_f64(),
        st_core::fmt::count((files as f64 / elapsed.as_secs_f64().max(0.001)) as u64),
        result.denied_count,
    );
    eprintln!(
        "exact bytes: logical={} alloc={}",
        tree.subtree_logical(result.root),
        tree.subtree_alloc(result.root)
    );

    let vol = volume::query(root).ok();
    let meta = ScanMeta {
        scanned_at: "now",
        engine: "Parallel walker (st-scan, portable)",
        duration: &format!("{:.2}s", elapsed.as_secs_f64()),
        volume: vol.as_ref(),
    };
    let sep = if cfg!(windows) { "\\" } else { "/" };
    let md = export_markdown(tree, result.root, sep, &meta, &ExportOptions::default());

    let out_path = "/tmp/spacetree-scan.md";
    std::fs::write(out_path, &md).expect("write markdown output");
    eprintln!("markdown report ({} bytes) written to {out_path}", md.len());
}
