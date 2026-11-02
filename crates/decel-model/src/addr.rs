// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable identifier of a sheet. Never reused within a workbook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SheetId(pub u32);

impl fmt::Display for SheetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Stable identifier of a chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChartId(pub u32);

/// A cell address. `row` and `col` are 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellAddr {
    pub sheet: SheetId,
    pub row: u32,
    pub col: u32,
}

impl CellAddr {
    pub const fn new(sheet: SheetId, row: u32, col: u32) -> Self {
        Self { sheet, row, col }
    }

    /// Moves by a signed offset. Returns `None` if the result is off the sheet.
    pub fn offset(self, dr: i64, dc: i64) -> Option<Self> {
        let row = u32::try_from(i64::from(self.row) + dr).ok()?;
        let col = u32::try_from(i64::from(self.col) + dc).ok()?;
        Some(Self { row, col, ..self })
    }
}

impl fmt::Display for CellAddr {
    /// Formats as the 1-based `row,col` used in the UI.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", u64::from(self.row) + 1, u64::from(self.col) + 1)
    }
}

/// An inclusive rectangle of cells on one sheet (0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub r1: u32,
    pub c1: u32,
    pub r2: u32,
    pub c2: u32,
}

impl Rect {
    /// Builds a rectangle from two corners in any order.
    pub fn new(ra: u32, ca: u32, rb: u32, cb: u32) -> Self {
        Self {
            r1: ra.min(rb),
            c1: ca.min(cb),
            r2: ra.max(rb),
            c2: ca.max(cb),
        }
    }

    pub fn cell(row: u32, col: u32) -> Self {
        Self::new(row, col, row, col)
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        (self.r1..=self.r2).contains(&row) && (self.c1..=self.c2).contains(&col)
    }

    pub fn rows(&self) -> u64 {
        u64::from(self.r2 - self.r1) + 1
    }

    pub fn cols(&self) -> u64 {
        u64::from(self.c2 - self.c1) + 1
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.r1 <= other.r2 && other.r1 <= self.r2 && self.c1 <= other.c2 && other.c1 <= self.c2
    }

    /// Grows the rectangle by `margin` cells on every side, saturating at 0.
    pub fn expand(&self, margin: u32) -> Self {
        Self {
            r1: self.r1.saturating_sub(margin),
            c1: self.c1.saturating_sub(margin),
            r2: self.r2.saturating_add(margin),
            c2: self.c2.saturating_add(margin),
        }
    }

    /// Row-major iteration over every address in the rectangle.
    pub fn iter(&self) -> impl Iterator<Item = (u32, u32)> + use<> {
        let Rect { r1, c1, r2, c2 } = *self;
        (r1..=r2).flat_map(move |r| (c1..=c2).map(move |c| (r, c)))
    }
}

/// Row or column axis, used by layout and structural operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Axis {
    Row,
    Col,
}

impl Axis {
    pub fn to_int(self) -> i64 {
        match self {
            Axis::Row => 0,
            Axis::Col => 1,
        }
    }

    pub fn from_int(v: i64) -> Option<Self> {
        match v {
            0 => Some(Axis::Row),
            1 => Some(Axis::Col),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_clamps_at_sheet_edge() {
        let a = CellAddr::new(SheetId(1), 0, 3);
        assert_eq!(a.offset(-1, 0), None);
        assert_eq!(a.offset(2, -3), Some(CellAddr::new(SheetId(1), 2, 0)));
    }

    #[test]
    fn rect_normalises_corners() {
        let r = Rect::new(5, 4, 1, 2);
        assert_eq!((r.r1, r.c1, r.r2, r.c2), (1, 2, 5, 4));
        assert_eq!(r.iter().count(), 15);
        assert!(r.contains(3, 3));
        assert!(!r.contains(0, 3));
    }

    #[test]
    fn display_is_one_based() {
        assert_eq!(CellAddr::new(SheetId(1), 0, 1).to_string(), "1,2");
    }
}
