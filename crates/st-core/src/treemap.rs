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
    let items: Vec<(NodeId, u64)> = tree
        .children(root)
        .iter()
        .map(|&id| {
            let size = if use_alloc {
                tree.subtree_alloc(id)
            } else {
                tree.subtree_logical(id)
            };
            (id, size)
        })
        .collect();
    layout(&items, area)
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
fn layout_row(row: &[(NodeId, f64)], area: Rect, result: &mut Vec<TreemapItem>) -> Rect {
    let row_area: f64 = row.iter().map(|&(_, a)| a).sum();
    if row.is_empty() || row_area <= 0.0 {
        return area;
    }

    if area.w <= area.h {
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
