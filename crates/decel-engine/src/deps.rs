// SPDX-License-Identifier: MIT OR Apache-2.0

//! Runtime-recorded dependency graph (DESIGN §6.1).
//!
//! Each evaluated cell's reads are recorded as single-cell edges, range
//! dependencies (kept in a per-sheet R-tree so "who depends on this cell?"
//! is a point query) and sheet-name dependencies. A cell's dependencies are
//! replaced wholesale every time it is evaluated.

use std::collections::{HashMap, HashSet};

use decel_model::{CellAddr, Rect, SheetId};
use rstar::primitives::{GeomWithData, Rectangle};
use rstar::{AABB, RTree};

type RangeEntry = GeomWithData<Rectangle<[i64; 2]>, CellAddr>;

fn rect_geom(r: Rect) -> Rectangle<[i64; 2]> {
    Rectangle::from_corners(
        [i64::from(r.r1), i64::from(r.c1)],
        [i64::from(r.r2), i64::from(r.c2)],
    )
}

/// What one evaluation read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Deps {
    pub cells: HashSet<CellAddr>,
    pub ranges: Vec<(SheetId, Rect)>,
    pub names: HashSet<String>,
}

impl Deps {
    pub fn cell(&mut self, a: CellAddr) {
        self.cells.insert(a);
    }

    pub fn range(&mut self, sheet: SheetId, rect: Rect) {
        if !self.ranges.contains(&(sheet, rect)) {
            self.ranges.push((sheet, rect));
        }
    }

    pub fn name(&mut self, name: &str) {
        if !self.names.contains(name) {
            self.names.insert(name.to_string());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.ranges.is_empty() && self.names.is_empty()
    }
}

#[derive(Default)]
pub struct DepGraph {
    forward: HashMap<CellAddr, Deps>,
    cell_rev: HashMap<CellAddr, HashSet<CellAddr>>,
    range_rev: HashMap<SheetId, RTree<RangeEntry>>,
    name_rev: HashMap<String, HashSet<CellAddr>>,
}

impl DepGraph {
    /// Replaces the dependencies of `of`.
    pub fn set(&mut self, of: CellAddr, deps: Deps) {
        self.remove(of);
        if deps.is_empty() {
            return;
        }
        for c in &deps.cells {
            self.cell_rev.entry(*c).or_default().insert(of);
        }
        for (sheet, rect) in &deps.ranges {
            self.range_rev
                .entry(*sheet)
                .or_default()
                .insert(RangeEntry::new(rect_geom(*rect), of));
        }
        for n in &deps.names {
            self.name_rev.entry(n.clone()).or_default().insert(of);
        }
        self.forward.insert(of, deps);
    }

    /// Forgets everything `of` depends on.
    pub fn remove(&mut self, of: CellAddr) {
        let Some(old) = self.forward.remove(&of) else {
            return;
        };
        for c in old.cells {
            if let Some(set) = self.cell_rev.get_mut(&c) {
                set.remove(&of);
                if set.is_empty() {
                    self.cell_rev.remove(&c);
                }
            }
        }
        for (sheet, rect) in old.ranges {
            if let Some(tree) = self.range_rev.get_mut(&sheet) {
                tree.remove(&RangeEntry::new(rect_geom(rect), of));
            }
        }
        for n in old.names {
            if let Some(set) = self.name_rev.get_mut(&n) {
                set.remove(&of);
                if set.is_empty() {
                    self.name_rev.remove(&n);
                }
            }
        }
    }

    pub fn deps_of(&self, of: CellAddr) -> Option<&Deps> {
        self.forward.get(&of)
    }

    /// Direct dependents of a change to `changed` (cell edges and ranges
    /// covering it).
    pub fn dependents(&self, changed: CellAddr, out: &mut Vec<CellAddr>) {
        if let Some(set) = self.cell_rev.get(&changed) {
            out.extend(set.iter().copied());
        }
        if let Some(tree) = self.range_rev.get(&changed.sheet) {
            let p = [i64::from(changed.row), i64::from(changed.col)];
            out.extend(tree.locate_all_at_point(p).map(|e| e.data));
        }
    }

    /// Dependents whose ranges intersect `rect` on `sheet`.
    pub fn dependents_in(&self, sheet: SheetId, rect: Rect, out: &mut Vec<CellAddr>) {
        if let Some(tree) = self.range_rev.get(&sheet) {
            let env = AABB::from_corners(
                [i64::from(rect.r1), i64::from(rect.c1)],
                [i64::from(rect.r2), i64::from(rect.c2)],
            );
            out.extend(tree.locate_in_envelope_intersecting(env).map(|e| e.data));
        }
        for (cell, deps) in &self.cell_rev {
            if cell.sheet == sheet && rect.contains(cell.row, cell.col) {
                out.extend(deps.iter().copied());
            }
        }
    }

    pub fn name_dependents(&self, name: &str, out: &mut Vec<CellAddr>) {
        if let Some(set) = self.name_rev.get(name) {
            out.extend(set.iter().copied());
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn len(&self) -> usize {
        self.forward.len()
    }

    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: SheetId = SheetId(1);

    fn a(r: u32, c: u32) -> CellAddr {
        CellAddr::new(S, r, c)
    }

    #[test]
    fn cell_range_and_name_dependents() {
        let mut g = DepGraph::default();
        let mut d = Deps::default();
        d.cell(a(0, 0));
        d.range(S, Rect::new(0, 1, u32::MAX, 1));
        d.name("Inputs");
        g.set(a(5, 5), d);

        let mut out = Vec::new();
        g.dependents(a(0, 0), &mut out);
        assert_eq!(out, [a(5, 5)]);
        out.clear();
        g.dependents(a(1_000_000, 1), &mut out);
        assert_eq!(out, [a(5, 5)]);
        out.clear();
        g.dependents(a(3, 2), &mut out);
        assert!(out.is_empty());
        g.name_dependents("Inputs", &mut out);
        assert_eq!(out, [a(5, 5)]);
    }

    #[test]
    fn set_replaces_and_remove_forgets() {
        let mut g = DepGraph::default();
        let mut d = Deps::default();
        d.cell(a(0, 0));
        d.range(S, Rect::new(0, 3, 9, 3));
        g.set(a(5, 5), d);
        let mut d2 = Deps::default();
        d2.cell(a(1, 1));
        g.set(a(5, 5), d2);
        let mut out = Vec::new();
        g.dependents(a(0, 0), &mut out);
        g.dependents(a(4, 3), &mut out);
        assert!(out.is_empty());
        g.dependents(a(1, 1), &mut out);
        assert_eq!(out, [a(5, 5)]);
        g.remove(a(5, 5));
        out.clear();
        g.dependents(a(1, 1), &mut out);
        assert!(out.is_empty());
        assert!(g.is_empty());
    }
}
