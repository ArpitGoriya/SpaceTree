//! The scan result: a struct-of-arrays tree, built in two passes.
//!
//! Engines emit unordered `(parent, name, size, flags)` records via
//! [`TreeBuilder`]. [`TreeBuilder::finalize`] then:
//! 1. counting-sorts nodes by parent into CSR layout so each node's
//!    children become one contiguous slice (no per-node `Vec<child>`
//!    allocation), and
//! 2. walks the tree bottom-up with an explicit stack (never recursion —
//!    a real filesystem can nest deeper than the default stack, and a
//!    corrupt volume could claim arbitrary depth) to roll up subtree size
//!    and file count.
//!
//! Everything is indexed by `u32` (`NodeId`), so a 2M-node tree costs
//! tens of MB rather than hundreds: no `Box`, no per-node `String`, no
//! pointer chasing.

use std::collections::HashMap;

use crate::NodeFlags;

pub type NodeId = u32;

/// Sentinel meaning "no parent" — used only for the synthetic root that
/// every [`TreeBuilder`] starts with.
pub const ROOT: NodeId = 0;

/// One record as emitted by a scan engine, before the tree is built.
pub struct RawNode {
    pub parent: NodeId,
    pub name: String,
    pub size_logical: u64,
    pub size_alloc: u64,
    pub mtime: i64,
    pub flags: NodeFlags,
}

/// Accumulates raw records from a scan engine into flat arrays. Not
/// thread-safe by design: parallel scan engines collect results on a
/// channel into a single collector thread that owns one `TreeBuilder`,
/// so a child's `parent` id is always already-assigned before the child
/// itself is pushed.
#[derive(Default)]
pub struct TreeBuilder {
    names: String,
    name_span: Vec<(u32, u16)>,
    parent: Vec<NodeId>,
    size_logical: Vec<u64>,
    size_alloc: Vec<u64>,
    mtime: Vec<i64>,
    flags: Vec<NodeFlags>,
}

impl TreeBuilder {
    pub fn new() -> Self {
        let mut b = Self::default();
        // Node 0 is always the synthetic root, pushed here so real
        // records start at id 1.
        b.push_raw(ROOT, "", 0, 0, 0, NodeFlags::DIR);
        b
    }

    /// Push one record, returning its assigned `NodeId`. `node.parent`
    /// must already be a valid id previously returned by this method (or
    /// [`ROOT`]) — an engine walking top-down naturally satisfies this
    /// since it creates a directory's node before scanning its contents.
    pub fn push(&mut self, node: RawNode) -> NodeId {
        self.push_raw(
            node.parent,
            &node.name,
            node.size_logical,
            node.size_alloc,
            node.mtime,
            node.flags,
        )
    }

    fn push_raw(
        &mut self,
        parent: NodeId,
        name: &str,
        size_logical: u64,
        size_alloc: u64,
        mtime: i64,
        flags: NodeFlags,
    ) -> NodeId {
        let id = self.parent.len() as NodeId;
        let offset = self.names.len() as u32;
        // Names are truncated at u16::MAX bytes; no real filename gets
        // remotely close (Windows caps components at 255 UTF-16 units).
        let len = name.len().min(u16::MAX as usize) as u16;
        self.names.push_str(&name[..len as usize]);
        self.name_span.push((offset, len));
        self.parent.push(parent);
        self.size_logical.push(size_logical);
        self.size_alloc.push(size_alloc);
        self.mtime.push(mtime);
        self.flags.push(flags);
        id
    }

    /// OR additional flags onto an already-pushed node — e.g. a
    /// directory only turns out to be unreadable once the engine tries
    /// to list it, which is necessarily after it was pushed as a plain
    /// `DIR` node.
    pub fn mark(&mut self, id: NodeId, extra: NodeFlags) {
        self.flags[id as usize].insert(extra);
    }

    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 1 // only the synthetic root
    }

    /// Consume the builder, sort children into contiguous CSR slices by
    /// parent, and roll up subtree totals. O(n) overall, single-threaded
    /// (measured at ~50ms for 2M nodes — not worth parallelizing).
    ///
    /// A record whose `parent` is out of range or equal to its own id
    /// (corrupt input — this can't happen from the directory walker,
    /// which assigns parents from ids it already issued, but a future
    /// engine that trusts on-disk records must not lose bytes silently
    /// or panic on one) is reattached under a synthetic `<unlinked>`
    /// node instead.
    pub fn finalize(self) -> Tree {
        let n = self.parent.len();

        let mut parent = self.parent;
        let mut names = self.names;
        let mut name_span = self.name_span;
        let mut size_logical = self.size_logical;
        let mut size_alloc = self.size_alloc;
        let mut mtime = self.mtime;
        let mut flags = self.flags;

        let has_orphans = (1..n).any(|i| {
            let p = parent[i] as usize;
            p >= n || p == i
        });
        let total = if has_orphans { n + 1 } else { n };
        let unlinked_id = n as NodeId;
        if has_orphans {
            let off = names.len() as u32;
            names.push_str("<unlinked>");
            name_span.push((off, "<unlinked>".len() as u16));
            parent.push(ROOT);
            size_logical.push(0);
            size_alloc.push(0);
            mtime.push(0);
            flags.push(NodeFlags::DIR);
            for i in 1..n {
                let p = parent[i] as usize;
                if p >= n || p == i {
                    parent[i] = unlinked_id;
                    flags[i].insert(NodeFlags::ORPHAN);
                }
            }
        }

        // Counting sort by parent into CSR layout: child_start[p] is the
        // offset into `order` where p's children begin, child_len[p] is
        // how many there are, and order[child_start[p]..][..child_len[p]]
        // holds their ids. Standard CSR construction — every non-root
        // node appears in `order` exactly once, as its parent's child.
        let mut child_len = vec![0u32; total];
        for &p in parent.iter().skip(1) {
            child_len[p as usize] += 1;
        }
        // Exclusive prefix sum of child_len: child_start[p] is where p's
        // children begin in `order`. Written as a running accumulator
        // over `zip` (rather than indexing child_start[i] and
        // child_len[i] directly) since it's a genuine sequential scan,
        // not the kind of independent per-index access needless_range_loop
        // warns about.
        let mut child_start = vec![0u32; total];
        let mut running = 0u32;
        for (start, &len) in child_start.iter_mut().skip(1).zip(child_len.iter()) {
            running += len;
            *start = running;
        }
        let mut cursor = child_start.clone();
        let mut order = vec![0u32; total - 1];
        for (i, &p) in parent.iter().enumerate().skip(1) {
            let p = p as usize;
            order[cursor[p] as usize] = i as NodeId;
            cursor[p] += 1;
        }

        let mut subtree_logical = size_logical.clone();
        let mut subtree_alloc = size_alloc.clone();
        let mut file_count = vec![0u32; total];
        for (fc, flag) in file_count.iter_mut().zip(flags.iter()).skip(1) {
            if !flag.contains(NodeFlags::DIR) {
                *fc = 1;
            }
        }

        // Iterative post-order rollup via explicit stack: push root,
        // pop+visit, push children, recording visit order; then fold in
        // reverse so every child is folded into its parent before the
        // parent itself is folded into the grandparent.
        let mut visit_order = Vec::with_capacity(total);
        let mut stack = vec![ROOT];
        while let Some(node) = stack.pop() {
            visit_order.push(node);
            let start = child_start[node as usize] as usize;
            let len = child_len[node as usize] as usize;
            stack.extend_from_slice(&order[start..start + len]);
        }
        debug_assert_eq!(
            visit_order.len(),
            total,
            "every node must be reachable from root; a scan engine emitted a parent \
             cycle that orphan detection didn't catch"
        );
        for &node in visit_order.iter().rev() {
            if node == ROOT {
                continue;
            }
            let p = parent[node as usize] as usize;
            subtree_logical[p] += subtree_logical[node as usize];
            subtree_alloc[p] += subtree_alloc[node as usize];
            file_count[p] += file_count[node as usize];
        }

        Tree {
            names,
            name_span,
            parent,
            child_start,
            child_len,
            order,
            size_logical,
            size_alloc,
            subtree_logical,
            subtree_alloc,
            file_count,
            mtime,
            flags,
        }
    }
}

/// A finalized, immutable scan tree. Children of any node are a
/// contiguous CSR slice, so lookups are O(1) and iteration is
/// cache-friendly.
#[derive(Debug)]
pub struct Tree {
    names: String,
    name_span: Vec<(u32, u16)>,
    parent: Vec<NodeId>,
    child_start: Vec<u32>,
    child_len: Vec<u32>,
    order: Vec<NodeId>,
    size_logical: Vec<u64>,
    size_alloc: Vec<u64>,
    subtree_logical: Vec<u64>,
    subtree_alloc: Vec<u64>,
    file_count: Vec<u32>,
    mtime: Vec<i64>,
    flags: Vec<NodeFlags>,
}

impl Tree {
    pub fn node_count(&self) -> usize {
        self.parent.len()
    }

    pub fn name(&self, id: NodeId) -> &str {
        let (off, len) = self.name_span[id as usize];
        &self.names[off as usize..off as usize + len as usize]
    }

    pub fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        if id == ROOT {
            None
        } else {
            Some(self.parent[id as usize])
        }
    }

    pub fn is_dir(&self, id: NodeId) -> bool {
        self.flags[id as usize].contains(NodeFlags::DIR)
    }

    pub fn flags(&self, id: NodeId) -> NodeFlags {
        self.flags[id as usize]
    }

    pub fn mtime(&self, id: NodeId) -> i64 {
        self.mtime[id as usize]
    }

    /// Own size (0 for directories; use [`Tree::subtree_logical`] for that).
    pub fn size_logical(&self, id: NodeId) -> u64 {
        self.size_logical[id as usize]
    }

    pub fn size_alloc(&self, id: NodeId) -> u64 {
        self.size_alloc[id as usize]
    }

    pub fn subtree_logical(&self, id: NodeId) -> u64 {
        self.subtree_logical[id as usize]
    }

    pub fn subtree_alloc(&self, id: NodeId) -> u64 {
        self.subtree_alloc[id as usize]
    }

    pub fn file_count(&self, id: NodeId) -> u32 {
        self.file_count[id as usize]
    }

    /// Children of `id`, in unspecified (insertion-derived) order. Sort
    /// them yourself for display — [`Tree::children_sorted_by_size`] is
    /// the common case.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        let start = self.child_start[id as usize] as usize;
        let len = self.child_len[id as usize] as usize;
        &self.order[start..start + len]
    }

    /// Children sorted largest-subtree-first, descending. Directories and
    /// files interleave by size, matching how WizTree-style tools present
    /// a folder's contents.
    pub fn children_sorted_by_size(&self, id: NodeId) -> Vec<NodeId> {
        let mut kids = self.children(id).to_vec();
        kids.sort_unstable_by(|&a, &b| {
            self.subtree_alloc[b as usize].cmp(&self.subtree_alloc[a as usize])
        });
        kids
    }

    /// Full path from root to `id`, joined with `sep` (e.g. `\` on
    /// Windows, `/` elsewhere). The synthetic root's own name is empty,
    /// so a bare drive scan renders as e.g. `C:\Users\...` rather than
    /// `\C:\Users\...`.
    pub fn path(&self, id: NodeId, sep: &str) -> String {
        let mut parts = Vec::new();
        let mut cur = id;
        loop {
            let name = self.name(cur);
            if !name.is_empty() {
                parts.push(name);
            }
            match self.parent_of(cur) {
                Some(p) => cur = p,
                None => break,
            }
        }
        parts.reverse();
        parts.join(sep)
    }

    /// Iterative depth-first walk of `root` and every descendant
    /// (`root` included, as the first item). No recursion, so export of
    /// an unbounded-depth subtree can't blow the stack.
    pub fn descendants(&self, root: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut stack = vec![root];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            stack.extend_from_slice(self.children(node));
            Some(node)
        })
    }

    /// Per-extension totals (lowercased, no dot) among file descendants
    /// of `root`, for the "by file type" export section. `root` itself
    /// is included in the walk but never counts, since it's a directory.
    pub fn extension_breakdown(&self, root: NodeId) -> HashMap<String, ExtStat> {
        let mut map: HashMap<String, ExtStat> = HashMap::new();
        for id in self.descendants(root) {
            if self.is_dir(id) {
                continue;
            }
            let name = self.name(id);
            let ext = match name.rsplit_once('.') {
                Some((_, e)) if !e.is_empty() => e.to_lowercase(),
                _ => "(none)".to_string(),
            };
            let entry = map.entry(ext).or_default();
            entry.logical += self.size_logical[id as usize];
            entry.alloc += self.size_alloc[id as usize];
            entry.files += 1;
        }
        map
    }
}

/// Totals for one file extension, as returned by [`Tree::extension_breakdown`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ExtStat {
    pub logical: u64,
    pub alloc: u64,
    pub files: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

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
            // simulate 4 KiB cluster rounding, like a real filesystem
            size_alloc: size.div_ceil(4096) * 4096,
            mtime: 0,
            flags: NodeFlags::empty(),
        }
    }

    /// Builds:
    /// ```text
    /// C:\                 (root child of the synthetic tree root)
    /// ├── Users
    /// │   └── a.txt   (1 byte  -> 4 KiB on disk)
    /// └── Windows
    ///     └── b.bin   (9000 bytes -> 12 KiB on disk)
    /// ```
    fn sample() -> (Tree, NodeId, NodeId, NodeId) {
        let mut b = TreeBuilder::new();
        let c = b.push(dir(ROOT, "C:"));
        let users = b.push(dir(c, "Users"));
        b.push(file(users, "a.txt", 1));
        let windows = b.push(dir(c, "Windows"));
        b.push(file(windows, "b.bin", 9000));
        (b.finalize(), c, users, windows)
    }

    #[test]
    fn rollup_sums_subtree_bytes_and_file_counts() {
        let (tree, c, users, windows) = sample();
        assert_eq!(tree.subtree_logical(users), 1);
        assert_eq!(tree.subtree_alloc(users), 4096);
        assert_eq!(tree.file_count(users), 1);

        assert_eq!(tree.subtree_logical(windows), 9000);
        assert_eq!(tree.subtree_alloc(windows), 12288);
        assert_eq!(tree.file_count(windows), 1);

        assert_eq!(tree.subtree_logical(c), 9001);
        assert_eq!(tree.subtree_alloc(c), 4096 + 12288);
        assert_eq!(tree.file_count(c), 2);
    }

    #[test]
    fn directories_have_zero_own_size() {
        let (tree, c, ..) = sample();
        assert_eq!(tree.size_logical(c), 0);
        assert_eq!(tree.size_alloc(c), 0);
    }

    #[test]
    fn children_are_returned_via_csr_slice() {
        let (tree, c, users, windows) = sample();
        let mut kids = tree.children(c).to_vec();
        kids.sort();
        let mut expected = vec![users, windows];
        expected.sort();
        assert_eq!(kids, expected);
    }

    #[test]
    fn children_sorted_by_size_is_largest_first() {
        let (tree, c, _users, windows) = sample();
        let sorted = tree.children_sorted_by_size(c);
        // Windows (12 KiB on disk) outweighs Users (4 KiB).
        assert_eq!(sorted[0], windows);
    }

    #[test]
    fn leaf_has_no_children() {
        let (tree, _c, users, _windows) = sample();
        let a_txt = tree.children(users)[0];
        assert!(tree.children(a_txt).is_empty());
        assert!(!tree.is_dir(a_txt));
    }

    #[test]
    fn path_joins_from_root_without_leading_separator() {
        let (tree, _c, users, _windows) = sample();
        let a_txt = tree.children(users)[0];
        assert_eq!(tree.path(a_txt, "\\"), "C:\\Users\\a.txt");
    }

    #[test]
    fn empty_builder_finalizes_to_a_bare_root() {
        let tree = TreeBuilder::new().finalize();
        assert_eq!(tree.node_count(), 1);
        assert_eq!(tree.subtree_logical(ROOT), 0);
        assert!(tree.children(ROOT).is_empty());
    }

    #[test]
    fn out_of_range_parent_is_reattached_under_unlinked_not_dropped() {
        let mut b = TreeBuilder::new();
        // Reference a parent id (42) that was never pushed.
        let orphan = b.push(file(42, "ghost.txt", 500));
        let tree = b.finalize();

        assert!(tree.flags(orphan).contains(NodeFlags::ORPHAN));
        let unlinked = tree.parent_of(orphan).expect("orphan must have a parent");
        assert_eq!(tree.name(unlinked), "<unlinked>");
        // The bytes are not lost: they still roll up into <unlinked>.
        assert_eq!(tree.subtree_logical(unlinked), 500);
        assert!(tree.children(ROOT).contains(&unlinked));
    }

    #[test]
    fn self_referential_parent_is_reattached_not_infinite_looped() {
        let mut b = TreeBuilder::new();
        // Simulate a corrupt record claiming itself as its own parent by
        // pushing with a parent id equal to the id it's about to receive.
        let next_id = b.len() as NodeId;
        let looped = b.push(dir(next_id, "weird"));
        assert_eq!(looped, next_id);
        let tree = b.finalize();
        assert!(tree.flags(looped).contains(NodeFlags::ORPHAN));
        assert_eq!(tree.name(tree.parent_of(looped).unwrap()), "<unlinked>");
    }

    #[test]
    fn extension_breakdown_is_scoped_to_the_given_root_not_the_whole_tree() {
        let mut b = TreeBuilder::new();
        let c = b.push(dir(ROOT, "C:"));
        let a = b.push(dir(c, "A"));
        b.push(file(a, "one.txt", 10));
        let outside = b.push(dir(c, "Outside"));
        b.push(file(outside, "two.txt", 20));
        let tree = b.finalize();

        let scoped = tree.extension_breakdown(a);
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped["txt"].files, 1);
        assert_eq!(scoped["txt"].logical, 10);
    }

    #[test]
    fn descendants_includes_root_itself() {
        let (tree, c, users, windows) = sample();
        let all: Vec<NodeId> = tree.descendants(c).collect();
        assert!(all.contains(&c));
        assert!(all.contains(&users));
        assert!(all.contains(&windows));
        assert_eq!(all.len(), 5); // C:, Users, a.txt, Windows, b.bin
    }
}
