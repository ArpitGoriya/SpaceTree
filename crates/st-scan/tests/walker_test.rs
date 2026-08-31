//! Integration tests for the portable walker, run against real temporary
//! directories — this is what actually proves the scan pipeline (readdir
//! -> per-entry metadata -> parallel dispatch -> single-collector tree
//! build -> rollup) works end to end, not just that it compiles.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use st_scan::scan;

fn write_file(path: &Path, bytes: usize) {
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

fn run(root: &Path) -> st_scan::ScanResult {
    let cancel = AtomicBool::new(false);
    scan(root, &cancel, |_| {}).expect("scan should succeed")
}

#[test]
fn scans_nested_directories_with_correct_totals() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("a")).unwrap();
    fs::create_dir(root.join("a/b")).unwrap();
    write_file(&root.join("a/one.txt"), 100);
    write_file(&root.join("a/b/two.txt"), 250);
    write_file(&root.join("top.bin"), 10);

    let result = run(root);
    let tree = &result.tree;

    assert_eq!(tree.subtree_logical(result.root), 360);
    assert_eq!(tree.file_count(result.root), 3);

    let a = tree
        .children(result.root)
        .iter()
        .find(|&&id| tree.name(id) == "a")
        .copied()
        .expect("'a' should be a child of root");
    assert_eq!(tree.subtree_logical(a), 350);
    assert_eq!(tree.file_count(a), 2);
    assert!(tree.is_dir(a));
}

#[test]
fn empty_directory_scans_to_a_bare_root() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path());
    assert_eq!(result.tree.subtree_logical(result.root), 0);
    assert_eq!(result.tree.file_count(result.root), 0);
    assert_eq!(result.denied_count, 0);
}

#[test]
fn scanning_a_file_instead_of_a_directory_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("not_a_dir.txt");
    write_file(&file, 5);
    let cancel = AtomicBool::new(false);
    let err = scan(&file, &cancel, |_| {}).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[cfg(unix)]
#[test]
fn symlinked_directory_is_recorded_but_never_traversed() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("real")).unwrap();
    write_file(&root.join("real/inside.txt"), 999);
    symlink(root.join("real"), root.join("link_to_real")).unwrap();

    let result = run(root);
    let tree = &result.tree;

    let link = tree
        .children(result.root)
        .iter()
        .find(|&&id| tree.name(id) == "link_to_real")
        .copied()
        .expect("the symlink itself must still be listed");
    assert!(tree.flags(link).contains(st_core::NodeFlags::REPARSE));
    // Never traversed: no children, and it contributes 0 bytes even
    // though the real directory behind it has 999 bytes in it.
    assert!(tree.children(link).is_empty());
    assert_eq!(tree.subtree_logical(link), 0);

    // The real directory, reached directly (not through the link), is
    // scanned normally and keeps its actual bytes.
    let real = tree
        .children(result.root)
        .iter()
        .find(|&&id| tree.name(id) == "real")
        .copied()
        .unwrap();
    assert_eq!(tree.subtree_logical(real), 999);
}

#[cfg(unix)]
#[test]
fn symlink_cycle_does_not_hang_the_scan() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("a")).unwrap();
    // a/loop -> root, which contains a, which contains loop, ... — a
    // walker that followed symlinks would recurse forever.
    symlink(root, root.join("a/loop")).unwrap();

    let result = run(root); // must return, not hang
    assert!(result.tree.node_count() >= 2);
}

#[cfg(unix)]
#[test]
fn hardlinked_file_is_counted_once() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_file(&root.join("first.dat"), 500);
    fs::hard_link(root.join("first.dat"), root.join("second.dat")).unwrap();

    let result = run(root);
    let tree = &result.tree;

    // 500 bytes total, not 1000 — the same on-disk data linked twice.
    assert_eq!(tree.subtree_logical(result.root), 500);
    // Both directory entries are still listed.
    assert_eq!(tree.children(result.root).len(), 2);

    let dup_count = tree
        .children(result.root)
        .iter()
        .filter(|&&id| tree.flags(id).contains(st_core::NodeFlags::HARDLINK_DUP))
        .count();
    assert_eq!(
        dup_count, 1,
        "exactly one of the two links is the flagged duplicate"
    );
}

#[test]
fn cancellation_before_scanning_returns_promptly_with_partial_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for i in 0..50 {
        let d = root.join(format!("dir{i}"));
        fs::create_dir(&d).unwrap();
        write_file(&d.join("f.txt"), 10);
    }

    let cancel = AtomicBool::new(true); // already cancelled
    let result = scan(root, &cancel, |_| {}).expect("cancellation is not an error");
    // Root itself is always present; deeper content may or may not be,
    // depending on exactly when workers observed the flag — the
    // contract is "returns promptly with whatever was found," not "finds
    // nothing."
    assert!(result.tree.node_count() >= 1);
}

/// Independent reference: sum file bytes and count files by directly
/// recursing with `std::fs`, deliberately not sharing any code with the
/// walker under test. Symlinks are skipped (matching "never followed").
fn reference_walk(dir: &Path) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let meta = entry.metadata().unwrap();
        if meta.is_symlink() {
            continue;
        } else if meta.is_dir() {
            let (b, f) = reference_walk(&entry.path());
            bytes += b;
            files += f;
        } else {
            bytes += meta.len();
            files += 1;
        }
    }
    (bytes, files)
}

#[test]
fn matches_an_independently_computed_reference_walk_of_a_real_directory() {
    // crates/st-core/src is real, non-trivial, and stable for the
    // duration of the test run — a good stand-in for "an actual
    // filesystem" without needing network or huge fixtures.
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../st-core/src");
    assert!(target.is_dir(), "fixture path must exist: {target:?}");

    let (expected_bytes, expected_files) = reference_walk(&target);
    let result = run(&target);

    assert_eq!(result.tree.subtree_logical(result.root), expected_bytes);
    assert_eq!(
        u64::from(result.tree.file_count(result.root)),
        expected_files
    );
}
