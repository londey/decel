// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sparse cell storage.

use std::collections::BTreeMap;

use crate::{Cell, Rect};

/// Storage for one sheet's non-empty cells. A trait so the representation
/// can be replaced later without touching callers.
pub trait CellStorage {
    fn get(&self, row: u32, col: u32) -> Option<&Cell>;
    /// Sets a cell, removing it if `cell` is empty. Returns the previous cell.
    fn set(&mut self, row: u32, col: u32, cell: Cell) -> Option<Cell>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The last row holding a cell, if any.
    fn last_row(&self) -> Option<u32>;
    /// The last column holding a cell, if any.
    fn last_col(&self) -> Option<u32>;
    /// All cells, row-major.
    fn iter(&self) -> Box<dyn Iterator<Item = ((u32, u32), &Cell)> + '_>;
    /// Cells inside `rect`, row-major.
    fn iter_rect(&self, rect: Rect) -> Box<dyn Iterator<Item = ((u32, u32), &Cell)> + '_>;
}

/// `BTreeMap`-backed sparse storage, keyed row-major.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SparseCells {
    cells: BTreeMap<(u32, u32), Cell>,
    /// Number of cells per column, so `last_col` is O(log n).
    col_counts: BTreeMap<u32, u32>,
}

impl SparseCells {
    pub fn new() -> Self {
        Self::default()
    }

    fn inc_col(&mut self, col: u32) {
        *self.col_counts.entry(col).or_insert(0) += 1;
    }

    fn dec_col(&mut self, col: u32) {
        if let Some(n) = self.col_counts.get_mut(&col) {
            *n -= 1;
            if *n == 0 {
                self.col_counts.remove(&col);
            }
        }
    }

    /// Removes every cell, returning them.
    pub fn take_all(&mut self) -> BTreeMap<(u32, u32), Cell> {
        self.col_counts.clear();
        std::mem::take(&mut self.cells)
    }
}

impl FromIterator<((u32, u32), Cell)> for SparseCells {
    fn from_iter<T: IntoIterator<Item = ((u32, u32), Cell)>>(iter: T) -> Self {
        let mut s = SparseCells::new();
        for ((r, c), cell) in iter {
            s.set(r, c, cell);
        }
        s
    }
}

impl CellStorage for SparseCells {
    fn get(&self, row: u32, col: u32) -> Option<&Cell> {
        self.cells.get(&(row, col))
    }

    fn set(&mut self, row: u32, col: u32, cell: Cell) -> Option<Cell> {
        let prev = if cell.is_empty() {
            self.cells.remove(&(row, col))
        } else {
            self.cells.insert((row, col), cell)
        };
        let now = self.cells.contains_key(&(row, col));
        match (prev.is_some(), now) {
            (false, true) => self.inc_col(col),
            (true, false) => self.dec_col(col),
            _ => {}
        }
        prev
    }

    fn len(&self) -> usize {
        self.cells.len()
    }

    fn last_row(&self) -> Option<u32> {
        self.cells.last_key_value().map(|((r, _), _)| *r)
    }

    fn last_col(&self) -> Option<u32> {
        self.col_counts.last_key_value().map(|(c, _)| *c)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = ((u32, u32), &Cell)> + '_> {
        Box::new(self.cells.iter().map(|(k, v)| (*k, v)))
    }

    fn iter_rect(&self, rect: Rect) -> Box<dyn Iterator<Item = ((u32, u32), &Cell)> + '_> {
        let Rect { r1, c1, r2, c2 } = rect;
        // Few rows relative to population: one range query per row. Otherwise
        // scan the row band and filter columns.
        if rect.rows() <= self.cells.len() as u64 {
            Box::new(
                (r1..=r2)
                    .flat_map(move |r| self.cells.range((r, c1)..=(r, c2)).map(|(k, v)| (*k, v))),
            )
        } else {
            Box::new(
                self.cells
                    .range((r1, c1)..=(r2, c2))
                    .filter(move |((_, c), _)| (c1..=c2).contains(c))
                    .map(|(k, v)| (*k, v)),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn v(i: i64) -> Cell {
        Cell::value(Value::Int(i))
    }

    #[test]
    fn set_get_remove() {
        let mut s = SparseCells::new();
        assert!(s.set(2, 3, v(1)).is_none());
        assert_eq!(s.get(2, 3), Some(&v(1)));
        assert_eq!(s.set(2, 3, Cell::EMPTY), Some(v(1)));
        assert!(s.is_empty());
        assert_eq!(s.last_col(), None);
    }

    #[test]
    fn last_row_and_col() {
        let mut s = SparseCells::new();
        s.set(5, 1, v(1));
        s.set(2, 9, v(2));
        s.set(5, 4, v(3));
        assert_eq!(s.last_row(), Some(5));
        assert_eq!(s.last_col(), Some(9));
        s.set(2, 9, Cell::EMPTY);
        assert_eq!(s.last_col(), Some(4));
    }

    #[test]
    fn rect_iteration_both_strategies() {
        let mut s = SparseCells::new();
        for r in 0..10 {
            for c in 0..10 {
                s.set(r, c, v(i64::from(r * 10 + c)));
            }
        }
        let small: Vec<_> = s.iter_rect(Rect::new(2, 3, 4, 4)).map(|(k, _)| k).collect();
        assert_eq!(small, vec![(2, 3), (2, 4), (3, 3), (3, 4), (4, 3), (4, 4)]);
        // Many more rows than cells forces the scan strategy.
        let tall: Vec<_> = s
            .iter_rect(Rect::new(8, 9, 1_000_000, 9))
            .map(|(k, _)| k)
            .collect();
        assert_eq!(tall, vec![(8, 9), (9, 9)]);
    }
}
