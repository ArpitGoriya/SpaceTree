//! Filename search over a scanned tree, scoped to a subtree — backs the
//! app's search bar ("substring and `*.ext` matching" per docs/PLAN.md).
//! Runs a linear scan of the name arena (documented in the plan as
//! ~15ms for 2M names), so no index is built or maintained.

use crate::tree::{NodeId, Tree};

/// Returns every descendant of `root` (root itself included) whose name
/// matches `query`, case-insensitive. A `query` containing `*` is
/// matched as a wildcard glob (`*` = any run of characters, including
/// none); anything else is a plain substring match.
pub fn search(tree: &Tree, root: NodeId, query: &str) -> Vec<NodeId> {
    if query.is_empty() {
        return Vec::new();
    }
    let is_glob = query.contains('*');
    let needle = query.to_lowercase();

    tree.descendants(root)
        .filter(|&id| {
            let name = tree.name(id).to_lowercase();
            if is_glob {
                glob_match(&needle, &name)
            } else {
                name.contains(&needle)
            }
        })
        .collect()
}

/// Wildcard match supporting `*` only (no `?`), both inputs already
/// lowercased by the caller. Classic two-pointer algorithm: track the
/// most recent `*` and the text position it last matched from, and
/// backtrack there on a mismatch instead of the usual recursive approach
/// (which would blow the stack on a long, heavily-starred pattern).
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None; // (pattern index after '*', text index it consumed up to)

    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi + 1, ti));
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some((sp, st)) = star {
            pi = sp;
            ti = st + 1;
            star = Some((sp, ti));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{RawNode, TreeBuilder, ROOT};
    use crate::NodeFlags;

    fn dir(b: &mut TreeBuilder, parent: NodeId, name: &str) -> NodeId {
        b.push(RawNode {
            parent,
            name: name.into(),
            size_logical: 0,
            size_alloc: 0,
            mtime: 0,
            flags: NodeFlags::DIR,
        })
    }
    fn file(b: &mut TreeBuilder, parent: NodeId, name: &str) -> NodeId {
        b.push(RawNode {
            parent,
            name: name.into(),
            size_logical: 1,
            size_alloc: 4096,
            mtime: 0,
            flags: NodeFlags::empty(),
        })
    }

    fn sample() -> (Tree, NodeId) {
        let mut b = TreeBuilder::new();
        let root = dir(&mut b, ROOT, "root");
        let videos = dir(&mut b, root, "Videos");
        file(&mut b, videos, "capture.mp4");
        file(&mut b, videos, "CAPTURE-2.MP4");
        file(&mut b, videos, "readme.txt");
        let docs = dir(&mut b, root, "Documents");
        file(&mut b, docs, "notes.txt");
        (b.finalize(), root)
    }

    #[test]
    fn empty_query_matches_nothing() {
        let (tree, root) = sample();
        assert!(search(&tree, root, "").is_empty());
    }

    #[test]
    fn substring_search_is_case_insensitive() {
        let (tree, root) = sample();
        let hits = search(&tree, root, "capture");
        assert_eq!(
            hits.len(),
            2,
            "should match both capture.mp4 and CAPTURE-2.MP4"
        );
    }

    #[test]
    fn extension_glob_matches_regardless_of_case() {
        let (tree, root) = sample();
        let hits = search(&tree, root, "*.mp4");
        assert_eq!(hits.len(), 2);
        for id in hits {
            assert!(tree.name(id).to_lowercase().ends_with(".mp4"));
        }
    }

    #[test]
    fn glob_with_wildcard_in_the_middle_matches() {
        let (tree, root) = sample();
        let hits = search(&tree, root, "cap*mp4");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_is_scoped_to_the_given_root() {
        let (tree, root) = sample();
        let videos = tree
            .children(root)
            .iter()
            .copied()
            .find(|&id| tree.name(id) == "Videos")
            .unwrap();
        // "notes.txt" lives under Documents, a sibling of Videos.
        assert!(search(&tree, videos, "notes").is_empty());
        assert_eq!(search(&tree, root, "notes").len(), 1);
    }

    #[test]
    fn plain_query_with_no_wildcard_does_not_match_as_glob() {
        let (tree, root) = sample();
        // "readme.txt" contains "me.t" as a literal substring.
        assert_eq!(search(&tree, root, "me.t").len(), 1);
    }
}
