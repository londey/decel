// SPDX-License-Identifier: MIT OR Apache-2.0

//! Operations (what a caller asks for) and changes (what actually happened,
//! with before and after images).

use serde::{Deserialize, Serialize};

use crate::{Axis, Cell, CellAddr, ChartId, MetaSource, SheetId};

/// A requested mutation. The store turns ops into [`Change`]s.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Sets cells. Later entries for the same address win.
    SetCells(Vec<(CellAddr, Cell)>),
    /// Adds a sheet at `position` (default: the end).
    AddSheet {
        name: String,
        position: Option<u32>,
    },
    RenameSheet {
        id: SheetId,
        name: String,
    },
    MoveSheet {
        id: SheetId,
        position: u32,
    },
    /// Deletes a sheet with its cells, layout and charts.
    DeleteSheet {
        id: SheetId,
    },
    /// Sets a row height or column width. `None` resets to the default.
    SetLayout {
        sheet: SheetId,
        axis: Axis,
        idx: u32,
        size: Option<f64>,
    },
    /// Creates (`id: None`) or replaces a chart.
    ChartUpsert {
        id: Option<ChartId>,
        chart: ChartRecord,
    },
    ChartDelete {
        id: ChartId,
    },
    /// Insert or delete rows or columns. Must be the only op in its txn.
    Structure(StructOp),
}

/// A transaction request: a label for history, the ops, and the sources of
/// any metatables the ops reference that may be new.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TxnInput {
    pub label: String,
    pub ops: Vec<Op>,
    pub metas: Vec<MetaSource>,
}

impl TxnInput {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    pub fn op(mut self, op: Op) -> Self {
        self.ops.push(op);
        self
    }

    pub fn meta(mut self, meta: MetaSource) -> Self {
        self.metas.push(meta);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetRecord {
    pub name: String,
    pub position: u32,
}

/// A chart: anchored to a cell, with an opaque JSON spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartRecord {
    pub sheet: SheetId,
    pub row: u32,
    pub col: u32,
    pub spec: String,
}

/// Row/column insertion or deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StructOp {
    InsertRows { sheet: SheetId, at: u32, n: u32 },
    DeleteRows { sheet: SheetId, at: u32, n: u32 },
    InsertCols { sheet: SheetId, at: u32, n: u32 },
    DeleteCols { sheet: SheetId, at: u32, n: u32 },
}

impl StructOp {
    pub fn inverse(self) -> Self {
        match self {
            StructOp::InsertRows { sheet, at, n } => StructOp::DeleteRows { sheet, at, n },
            StructOp::DeleteRows { sheet, at, n } => StructOp::InsertRows { sheet, at, n },
            StructOp::InsertCols { sheet, at, n } => StructOp::DeleteCols { sheet, at, n },
            StructOp::DeleteCols { sheet, at, n } => StructOp::InsertCols { sheet, at, n },
        }
    }

    pub fn sheet(self) -> SheetId {
        match self {
            StructOp::InsertRows { sheet, .. }
            | StructOp::DeleteRows { sheet, .. }
            | StructOp::InsertCols { sheet, .. }
            | StructOp::DeleteCols { sheet, .. } => sheet,
        }
    }

    pub fn axis(self) -> Axis {
        match self {
            StructOp::InsertRows { .. } | StructOp::DeleteRows { .. } => Axis::Row,
            StructOp::InsertCols { .. } | StructOp::DeleteCols { .. } => Axis::Col,
        }
    }

    pub fn is_delete(self) -> bool {
        matches!(
            self,
            StructOp::DeleteRows { .. } | StructOp::DeleteCols { .. }
        )
    }

    /// `(at, n)`.
    pub fn span(self) -> (u32, u32) {
        match self {
            StructOp::InsertRows { at, n, .. }
            | StructOp::DeleteRows { at, n, .. }
            | StructOp::InsertCols { at, n, .. }
            | StructOp::DeleteCols { at, n, .. } => (at, n),
        }
    }

    /// Maps an index on this op's axis forward through the op. Returns
    /// `None` for an index inside a deleted span.
    pub fn map_index(self, idx: u32) -> Option<u32> {
        let (at, n) = self.span();
        if self.is_delete() {
            if idx < at {
                Some(idx)
            } else if idx < at.saturating_add(n) {
                None
            } else {
                Some(idx - n)
            }
        } else if idx < at {
            Some(idx)
        } else {
            idx.checked_add(n)
        }
    }

    /// Maps an address forward through the op (other sheets are unchanged).
    pub fn map_addr(self, addr: CellAddr) -> Option<CellAddr> {
        if addr.sheet != self.sheet() {
            return Some(addr);
        }
        match self.axis() {
            Axis::Row => Some(CellAddr {
                row: self.map_index(addr.row)?,
                ..addr
            }),
            Axis::Col => Some(CellAddr {
                col: self.map_index(addr.col)?,
                ..addr
            }),
        }
    }
}

/// One recorded change with before and after images.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Cell {
        addr: CellAddr,
        before: Cell,
        after: Cell,
    },
    Sheet {
        id: SheetId,
        before: Option<SheetRecord>,
        after: Option<SheetRecord>,
    },
    Layout {
        sheet: SheetId,
        axis: Axis,
        idx: u32,
        before: Option<f64>,
        after: Option<f64>,
    },
    Chart {
        id: ChartId,
        before: Option<ChartRecord>,
        after: Option<ChartRecord>,
    },
    Structure(StructOp),
}

impl Change {
    /// The change that undoes this one.
    pub fn inverse(&self) -> Change {
        match self.clone() {
            Change::Cell {
                addr,
                before,
                after,
            } => Change::Cell {
                addr,
                before: after,
                after: before,
            },
            Change::Sheet { id, before, after } => Change::Sheet {
                id,
                before: after,
                after: before,
            },
            Change::Layout {
                sheet,
                axis,
                idx,
                before,
                after,
            } => Change::Layout {
                sheet,
                axis,
                idx,
                before: after,
                after: before,
            },
            Change::Chart { id, before, after } => Change::Chart {
                id,
                before: after,
                after: before,
            },
            Change::Structure(op) => Change::Structure(op.inverse()),
        }
    }

    /// Whether applying this change would do nothing.
    pub fn is_noop(&self) -> bool {
        match self {
            Change::Cell { before, after, .. } => before == after,
            Change::Sheet { before, after, .. } => before == after,
            Change::Layout { before, after, .. } => before == after,
            Change::Chart { before, after, .. } => before == after,
            Change::Structure(op) => op.span().1 == 0,
        }
    }
}

/// Inverts an ordered list of changes: reversed order, each inverted.
pub fn invert_changes(changes: &[Change]) -> Vec<Change> {
    changes.iter().rev().map(Change::inverse).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_index_through_ops() {
        let s = SheetId(1);
        let ins = StructOp::InsertRows {
            sheet: s,
            at: 5,
            n: 3,
        };
        assert_eq!(ins.map_index(4), Some(4));
        assert_eq!(ins.map_index(5), Some(8));
        let del = StructOp::DeleteRows {
            sheet: s,
            at: 5,
            n: 3,
        };
        assert_eq!(del.map_index(4), Some(4));
        assert_eq!(del.map_index(6), None);
        assert_eq!(del.map_index(8), Some(5));
    }

    #[test]
    fn struct_op_json() {
        let op = StructOp::InsertRows {
            sheet: SheetId(1),
            at: 10,
            n: 3,
        };
        let json = serde_json::to_string(&op).unwrap();
        assert_eq!(json, r#"{"op":"insert_rows","sheet":1,"at":10,"n":3}"#);
    }
}
