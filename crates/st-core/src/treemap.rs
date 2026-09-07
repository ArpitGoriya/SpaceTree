//! Squarified treemap layout (Bruls, Huizing & van Wijk, 2000): lays out
//! a set of sized items to tile a rectangle while keeping each item's
//! aspect ratio as close to square as possible, which is what makes a
//! treemap readable at a glance instead of a strip of slivers.
//!
//! This module only lays out *one level* — the direct children of
//! whatever node the caller passes in. The UI's "click to zoom" model
//! (see docs/PLAN.md) replaces the whole view with a fresh layout of the
//! clicked node's own children rather than nesting rects recursively, so
//! one level is all the layout math needs to produce.

use crate::tree::{NodeId, Tree};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn area(&self) -> f64 {
        self.w * self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreemapItem {
    pub node: NodeId,
    pub rect: Rect,
}

/// Lays out `items` (unsorted, zero-size entries are dropped since they'd
/// produce degenerate rects) to tile `area`. Empty input or a
/// non-positive area yields an empty result rather than dividing by zero.
pub fn layout(items: &[(NodeId, u64)], area: Rect) -> Vec<TreemapItem> {
    let total: u128 = items.iter().map(|&(_, s)| s as u128).sum();
    if total == 0 || area.w <= 0.0 || area.h <= 0.0 {
        return Vec::new();
    }

    let scale = area.area() / total as f64;
    let mut scaled: Vec<(NodeId, f64)> = items
        .iter()
        .filter(|&&(_, s)| s > 0)
        .map(|&(id, s)| (id, s as f64 * scale))
        .collect();
    // Squarify assumes descending order — that's what keeps each row's
    // aspect ratio close to square instead of alternating tiny/huge items.
    scaled.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    let mut result = Vec::with_capacity(scaled.len());
    let mut row: Vec<(NodeId, f64)> = Vec::new();
    squarify(&scaled, &mut row, area, &mut result);
    result
}

/// Convenience entry point for the app: lays out `root`'s direct
/// children by on-disk or logical subtree size.
pub fn layout_children(tree: &Tree, root: NodeId, area: Rect, use_alloc: bool) -> Vec<TreemapItem> {
    layout(&child_sizes(tree, root, use_alloc), area)
}

fn child_sizes(tree: &Tree, root: NodeId, use_alloc: bool) -> Vec<(NodeId, u64)> {
    tree.live_children(root)
        .iter()
        .map(|&id| {
            let size = if use_alloc {
                tree.subtree_alloc(id)
            } else {
                tree.subtree_logical(id)
            };
            (id, size)
        })
        .collect()
}

/// What a laid-out rectangle stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreemapCell {
    Node(NodeId),
    /// Everything too small to be worth its own rectangle, folded into
    /// one. Carries what was folded so the UI can say so rather than
    /// silently dropping it.
    Aggregate {
        count: u32,
        bytes: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreemapEntry {
    pub cell: TreemapCell,
    pub rect: Rect,
}

/// Lays out `root`'s children, folding the ones too small to draw into a
/// single trailing rectangle.
///
/// Without this a folder with thousands of children produces thousands of
/// sub-pixel rects: invisible once the 1px gutter is subtracted, too short
/// to carry a label, yet still serialized over IPC on every resize frame
/// and still hit-tested under the cursor. Folding them frees that area for
/// the folders that actually matter and bounds the payload, which is what
/// docs/PLAN.md asked for.
///
/// `min_cell_area` is in the same units as `area` (CSS pixels²);
/// `max_cells` bounds the result regardless of how the areas fall out.
/// The fold only happens when it would combine more than one child —
/// replacing a single small rect with a single "1 smaller item" rect
/// would lose its name for nothing.
pub fn layout_children_aggregated(
    tree: &Tree,
    root: NodeId,
    area: Rect,
    use_alloc: bool,
    min_cell_area: f64,
    max_cells: usize,
) -> Vec<TreemapEntry> {
    let mut children = child_sizes(tree, root, use_alloc);
    children.retain(|&(_, size)| size > 0);
    children.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let total: u128 = children.iter().map(|&(_, s)| s as u128).sum();
    if total == 0 || area.w <= 0.0 || area.h <= 0.0 {
        return Vec::new();
    }

    // How many of the largest children earn their own rect: those still
    // above the area floor, and no more than the cell budget leaves room
    // for. Children are already sorted descending, so this is a prefix.
    let canvas = area.area();
    let big_enough = children
        .iter()
        .take_while(|&&(_, size)| (size as f64 / total as f64) * canvas >= min_cell_area)
        .count();
    let keep = big_enough.min(max_cells.saturating_sub(1));

    let folded = &children[keep..];
    // One folded child would just be itself under a worse name.
    let (kept, aggregate) = if folded.len() > 1 {
        let bytes: u64 = folded.iter().map(|&(_, s)| s).sum();
        (&children[..keep], Some((folded.len() as u32, bytes)))
    } else {
        (&children[..], None)
    };

    // `layout` keys on NodeId, so feed it positions in `cells` instead of
    // real ids — that keeps the synthetic aggregate addressable without
    // reserving a sentinel NodeId that a real tree could one day collide
    // with.
    let mut cells: Vec<TreemapCell> = kept.iter().map(|&(id, _)| TreemapCell::Node(id)).collect();
    let mut sized: Vec<(NodeId, u64)> = kept
        .iter()
        .enumerate()
        .map(|(i, &(_, size))| (i as NodeId, size))
        .collect();
    if let Some((count, bytes)) = aggregate {
        sized.push((cells.len() as NodeId, bytes));
        cells.push(TreemapCell::Aggregate { count, bytes });
    }

    layout(&sized, area)
        .into_iter()
        .map(|item| TreemapEntry {
            cell: cells[item.node as usize],
            rect: item.rect,
        })
        .collect()
}

fn shorter_side(r: Rect) -> f64 {
    r.w.min(r.h)
}

/// Closed-form worst aspect ratio for a candidate row, from the paper:
/// `max(side² · max(row) / sum(row)², sum(row)² / (side² · min(row)))`.
/// Lower is better (1.0 is a perfect square); this is what squarify
/// minimizes one item at a time.
fn worst_ratio(row: &[f64], side: f64) -> f64 {
    let sum: f64 = row.iter().sum();
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let side2 = side * side;
    let sum2 = sum * sum;
    (side2 * max / sum2).max(sum2 / (side2 * min))
}

fn squarify(
    items: &[(NodeId, f64)],
    row: &mut Vec<(NodeId, f64)>,
    area: Rect,
    result: &mut Vec<TreemapItem>,
) {
    let Some((&first, rest)) = items.split_first() else {
        layout_row(row, area, result);
        return;
    };

    let side = shorter_side(area);
    let row_areas: Vec<f64> = row.iter().map(|&(_, a)| a).collect();
    let mut candidate_areas = row_areas.clone();
    candidate_areas.push(first.1);

    if row.is_empty() || worst_ratio(&row_areas, side) >= worst_ratio(&candidate_areas, side) {
        row.push(first);
        squarify(rest, row, area, result);
    } else {
        let remaining = layout_row(row, area, result);
        row.clear();
        squarify(items, row, remaining, result);
    }
}

/// Places `row` as a strip along the shorter side of `area` (vertical
/// strip on the left if `area` is wider than tall, horizontal strip on
/// top otherwise), and returns whatever rectangle is left over.
///
/// The "shorter side" is not a detail: [`worst_ratio`] scores a candidate
/// row against `shorter_side(area)`, so a strip laid along the *longer*
/// side means the admission test is judging a geometry that never gets
/// drawn. It then concludes "one item per row", and that one item is
/// stretched across the full long dimension — a stripe. The leftover rect
/// keeps the long dimension intact, so every following row is thinner
/// than the last and the whole thing degenerates into slice-and-dice.
/// This module got that backwards once already; `rects_stay_close_to_square_at_any_aspect_ratio`
/// is the test that pins it down.
fn layout_row(row: &[(NodeId, f64)], area: Rect, result: &mut Vec<TreemapItem>) -> Rect {
    let row_area: f64 = row.iter().map(|&(_, a)| a).sum();
    if row.is_empty() || row_area <= 0.0 {
        return area;
    }

    if area.w > area.h {
        // Wider than tall: the short side is the height, so the row is a
        // full-height column on the left and its items stack downward.
        let strip_w = row_area / area.h;
        let mut y = area.y;
        for &(id, a) in row {
            let h = a / strip_w;
            result.push(TreemapItem {
                node: id,
                rect: Rect {
                    x: area.x,
                    y,
                    w: strip_w,
                    h,
                },
            });
            y += h;
        }
        Rect {
            x: area.x + strip_w,
            y: area.y,
            w: (area.w - strip_w).max(0.0),
            h: area.h,
        }
    } else {
        // Taller than wide: the short side is the width, so the row is a
        // full-width band on top and its items run left to right.
        let strip_h = row_area / area.w;
        let mut x = area.x;
        for &(id, a) in row {
            let w = a / strip_h;
            result.push(TreemapItem {
                node: id,
                rect: Rect {
                    x,
                    y: area.y,
                    w,
                    h: strip_h,
                },
            });
            x += w;
        }
        Rect {
            x: area.x,
            y: area.y + strip_h,
            w: area.w,
            h: (area.h - strip_h).max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1000.0,
        h: 600.0,
    };

    fn total_rect_area(items: &[TreemapItem]) -> f64 {
        items.iter().map(|it| it.rect.area()).sum()
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert!(layout(&[], CANVAS).is_empty());
    }

    /// The property that *defines* a squarified treemap, and the one this
    /// module went without for its first two releases: rects stay near
    /// square regardless of the container's shape.
    ///
    /// Every other test here passes just as happily under slice-and-dice,
    /// because slicing conserves area, keeps rects inside the canvas and
    /// splits two equal items in half — so none of them noticed that
    /// `layout_row` was laying strips along the long side. That produced
    /// full-span stripes with a worst aspect ratio of ~1566:1 on a
    /// 1200x200 pane (and ~522:1 even on a friendly 1200x600 one), which
    /// is what a user reported seeing.
    ///
    /// The bound is deliberately loose: squarify is a greedy heuristic and
    /// a long tail of small items genuinely cannot all be square. It only
    /// has to be tight enough to catch a degenerate strip, and anything
    /// above ~8:1 is unreadable in practice.
    #[test]
    fn rects_stay_close_to_square_at_any_aspect_ratio() {
        const MAX_ACCEPTABLE_RATIO: f64 = 8.0;
        let sizes: Vec<(NodeId, u64)> = [4000u64, 2500, 1800, 900, 600, 300, 150, 90, 60, 40]
            .iter()
            .enumerate()
            .map(|(i, &s)| (i as NodeId, s))
            .collect();

        for (w, h) in [
            (1200.0, 200.0), // wide and short — the reported case
            (200.0, 1200.0), // its transpose
            (1200.0, 600.0),
            (600.0, 600.0),
        ] {
            let area = Rect {
                x: 0.0,
                y: 0.0,
                w,
                h,
            };
            for item in layout(&sizes, area) {
                let ratio = (item.rect.w / item.rect.h).max(item.rect.h / item.rect.w);
                assert!(
                    ratio <= MAX_ACCEPTABLE_RATIO,
                    "node {} is a {:.0}:1 sliver in a {w}x{h} area \
                     ({:.1}x{:.1}) — the layout has degenerated into strips",
                    item.node,
                    ratio,
                    item.rect.w,
                    item.rect.h,
                );
            }
        }
    }

    /// Builds a tree whose root has `n` children: a few large ones and a
    /// long tail of 1-byte ones, which is the shape that made the treemap
    /// unreadable.
    fn root_with_tail(large: &[u64], tail: usize) -> (Tree, NodeId) {
        use crate::tree::{RawNode, TreeBuilder, ROOT};
        use crate::NodeFlags;

        let leaf = |parent: NodeId, name: &str, size: u64| RawNode {
            parent,
            name: name.to_string(),
            size_logical: size,
            size_alloc: size,
            mtime: 0,
            flags: NodeFlags::empty(),
        };

        let mut b = TreeBuilder::new();
        let root = b.push(RawNode {
            parent: ROOT,
            name: "root".to_string(),
            size_logical: 0,
            size_alloc: 0,
            mtime: 0,
            flags: NodeFlags::DIR,
        });
        for (i, &size) in large.iter().enumerate() {
            b.push(leaf(root, &format!("big{i}"), size));
        }
        for i in 0..tail {
            b.push(leaf(root, &format!("tiny{i}"), 1));
        }
        (b.finalize(), root)
    }

    #[test]
    fn a_long_tail_of_tiny_children_folds_into_one_cell() {
        let (tree, root) = root_with_tail(&[8_000_000, 4_000_000, 1_000_000], 900);
        let entries = layout_children_aggregated(&tree, root, CANVAS, true, 120.0, 250);

        let aggregates: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e.cell, TreemapCell::Aggregate { .. }))
            .collect();
        assert_eq!(aggregates.len(), 1, "expected exactly one folded cell");
        assert!(
            matches!(
                aggregates[0].cell,
                TreemapCell::Aggregate {
                    count: 900,
                    bytes: 900
                }
            ),
            "the folded cell must report everything it stands for, got {:?}",
            aggregates[0].cell
        );
        assert_eq!(
            entries.len(),
            4,
            "three real folders plus the folded cell — the 900 tiny ones must not each get a rect"
        );
    }

    #[test]
    fn the_cell_budget_is_never_exceeded() {
        let (tree, root) = root_with_tail(&[100; 400], 0);
        let entries = layout_children_aggregated(&tree, root, CANVAS, true, 0.0, 50);
        assert!(entries.len() <= 50, "got {} cells", entries.len());
    }

    #[test]
    fn nothing_is_folded_when_every_child_is_big_enough() {
        let (tree, root) = root_with_tail(&[500, 400, 300], 0);
        let entries = layout_children_aggregated(&tree, root, CANVAS, true, 1.0, 250);
        assert_eq!(entries.len(), 3);
        assert!(entries
            .iter()
            .all(|e| matches!(e.cell, TreemapCell::Node(_))));
    }

    #[test]
    fn a_single_small_child_keeps_its_own_identity() {
        // Folding one item would replace a named folder with an anonymous
        // "1 smaller item" and gain nothing.
        let (tree, root) = root_with_tail(&[10_000_000, 5_000_000], 1);
        let entries = layout_children_aggregated(&tree, root, CANVAS, true, 120.0, 250);
        assert!(
            entries
                .iter()
                .all(|e| matches!(e.cell, TreemapCell::Node(_))),
            "a lone small child must not be folded: {entries:?}"
        );
    }

    #[test]
    fn folding_still_tiles_the_whole_canvas() {
        let (tree, root) = root_with_tail(&[8_000_000, 4_000_000], 500);
        let entries = layout_children_aggregated(&tree, root, CANVAS, true, 120.0, 250);
        let covered: f64 = entries.iter().map(|e| e.rect.area()).sum();
        assert!(
            (covered - CANVAS.area()).abs() < 1.0,
            "folded layout must still cover the canvas: {covered} vs {}",
            CANVAS.area()
        );
    }

    /// A row must be cut off the container's *long* dimension, so the
    /// leftover trends toward square instead of getting ever more extreme.
    /// Cutting the short dimension is what made the old code's stripes
    /// compound row after row.
    #[test]
    fn a_wide_area_is_divided_by_vertical_cuts() {
        let items = layout(
            &[(1, 50), (2, 30), (3, 20)],
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1200.0,
                h: 200.0,
            },
        );
        // The largest item alone in a full-height column is the only way
        // to keep it near square here; a full-width band would be 6:1.
        let first = items.iter().find(|it| it.node == 1).unwrap();
        assert!(
            (first.rect.h - 200.0).abs() < 1e-6,
            "expected a full-height column, got {:?}",
            first.rect
        );
        assert!(first.rect.w < 1200.0);
    }

    #[test]
    fn all_zero_sizes_yields_empty_output_not_a_panic() {
        assert!(layout(&[(1, 0), (2, 0)], CANVAS).is_empty());
    }

    #[test]
    fn non_positive_area_yields_empty_output() {
        let degenerate = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 600.0,
        };
        assert!(layout(&[(1, 100)], degenerate).is_empty());
    }

    #[test]
    fn single_item_fills_the_entire_area_exactly() {
        let items = layout(&[(1, 500)], CANVAS);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].rect, CANVAS);
    }

    #[test]
    fn zero_size_items_are_dropped_but_others_still_tile_the_area() {
        let items = layout(&[(1, 500), (2, 0), (3, 500)], CANVAS);
        assert_eq!(
            items.len(),
            2,
            "the zero-size entry must not produce a rect"
        );
        assert!((total_rect_area(&items) - CANVAS.area()).abs() < 1e-6);
    }

    #[test]
    fn rects_conserve_total_area_for_many_varied_sizes() {
        let sizes: Vec<(NodeId, u64)> = vec![
            (1, 4000),
            (2, 2500),
            (3, 1800),
            (4, 900),
            (5, 600),
            (6, 300),
            (7, 150),
            (8, 90),
        ];
        let items = layout(&sizes, CANVAS);
        assert_eq!(items.len(), sizes.len());
        assert!(
            (total_rect_area(&items) - CANVAS.area()).abs() < 1e-6,
            "sum of rect areas must equal the container area, got {} vs {}",
            total_rect_area(&items),
            CANVAS.area()
        );
    }

    #[test]
    fn every_rect_has_positive_dimensions_and_stays_inside_the_canvas() {
        let sizes: Vec<(NodeId, u64)> = (1..=12).map(|i| (i, (13 - i) as u64 * 37)).collect();
        for it in layout(&sizes, CANVAS) {
            assert!(
                it.rect.w > 0.0 && it.rect.h > 0.0,
                "degenerate rect: {:?}",
                it.rect
            );
            assert!(
                it.rect.x >= CANVAS.x - 1e-6 && it.rect.x + it.rect.w <= CANVAS.x + CANVAS.w + 1e-6
            );
            assert!(
                it.rect.y >= CANVAS.y - 1e-6 && it.rect.y + it.rect.h <= CANVAS.y + CANVAS.h + 1e-6
            );
        }
    }

    #[test]
    fn two_equal_items_split_the_canvas_in_half() {
        let items = layout(&[(1, 100), (2, 100)], CANVAS);
        assert_eq!(items.len(), 2);
        for it in &items {
            assert!((it.rect.area() - CANVAS.area() / 2.0).abs() < 1e-6);
        }
    }

    #[test]
    fn layout_children_uses_subtree_size_and_respects_the_alloc_toggle() {
        use crate::tree::{RawNode, TreeBuilder, ROOT};
        use crate::NodeFlags;

        let mut b = TreeBuilder::new();
        let root = b.push(RawNode {
            parent: ROOT,
            name: "root".into(),
            size_logical: 0,
            size_alloc: 0,
            mtime: 0,
            flags: NodeFlags::DIR,
        });
        b.push(RawNode {
            parent: root,
            name: "a".into(),
            size_logical: 100,
            size_alloc: 4096,
            mtime: 0,
            flags: NodeFlags::empty(),
        });
        b.push(RawNode {
            parent: root,
            name: "b".into(),
            size_logical: 300,
            size_alloc: 4096,
            mtime: 0,
            flags: NodeFlags::empty(),
        });
        let tree = b.finalize();

        // On-disk: both children are cluster-rounded to the same 4 KiB,
        // so they should come out equal-area despite differing logical sizes.
        let alloc_items = layout_children(&tree, root, CANVAS, true);
        assert_eq!(alloc_items.len(), 2);
        assert!((alloc_items[0].rect.area() - alloc_items[1].rect.area()).abs() < 1e-6);

        // Logical: 100 vs 300 bytes must produce a 1:3 area split.
        let logical_items = layout_children(&tree, root, CANVAS, false);
        let areas: Vec<f64> = logical_items.iter().map(|it| it.rect.area()).collect();
        let (small, large) = if areas[0] < areas[1] {
            (areas[0], areas[1])
        } else {
            (areas[1], areas[0])
        };
        assert!((large / small - 3.0).abs() < 1e-6);
    }
}
