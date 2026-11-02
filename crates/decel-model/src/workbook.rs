// SPDX-License-Identifier: MIT OR Apache-2.0

//! The in-memory materialised workbook.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::{
    Axis, Cell, CellAddr, CellStorage, Change, ChartId, ChartRecord, MetaHash, MetaSource, SheetId,
    SheetRecord, SparseCells, StructOp,
};

/// Row heights and column widths that differ from the default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub rows: BTreeMap<u32, f64>,
    pub cols: BTreeMap<u32, f64>,
}

impl Layout {
    pub fn axis(&self, axis: Axis) -> &BTreeMap<u32, f64> {
        match axis {
            Axis::Row => &self.rows,
            Axis::Col => &self.cols,
        }
    }

    pub fn axis_mut(&mut self, axis: Axis) -> &mut BTreeMap<u32, f64> {
        match axis {
            Axis::Row => &mut self.rows,
            Axis::Col => &mut self.cols,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub id: SheetId,
    pub name: String,
    pub position: u32,
    pub cells: SparseCells,
    pub layout: Layout,
}

impl Sheet {
    pub fn new(id: SheetId, name: impl Into<String>, position: u32) -> Self {
        Self {
            id,
            name: name.into(),
            position,
            cells: SparseCells::new(),
            layout: Layout::default(),
        }
    }

    pub fn record(&self) -> SheetRecord {
        SheetRecord {
            name: self.name.clone(),
            position: self.position,
        }
    }
}

/// The current state of a workbook.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workbook {
    pub sheets: BTreeMap<SheetId, Sheet>,
    pub metas: HashMap<MetaHash, Arc<MetaSource>>,
    pub charts: BTreeMap<ChartId, ChartRecord>,
    pub settings: BTreeMap<String, String>,
}

impl Workbook {
    /// Sheets in tab order.
    pub fn sheets_ordered(&self) -> Vec<&Sheet> {
        let mut v: Vec<&Sheet> = self.sheets.values().collect();
        v.sort_by_key(|s| (s.position, s.id));
        v
    }

    pub fn sheet(&self, id: SheetId) -> Option<&Sheet> {
        self.sheets.get(&id)
    }

    pub fn sheet_by_name(&self, name: &str) -> Option<&Sheet> {
        self.sheets.values().find(|s| s.name == name)
    }

    pub fn cell(&self, addr: CellAddr) -> Option<&Cell> {
        self.sheets.get(&addr.sheet)?.cells.get(addr.row, addr.col)
    }

    pub fn meta(&self, hash: &MetaHash) -> Option<&Arc<MetaSource>> {
        self.metas.get(hash)
    }

    pub fn add_meta(&mut self, src: MetaSource) -> MetaHash {
        let hash = src.hash();
        self.metas.entry(hash).or_insert_with(|| Arc::new(src));
        hash
    }

    /// Applies one change's after-image.
    pub fn apply_change(&mut self, change: &Change) {
        match change {
            Change::Cell { addr, after, .. } => {
                if let Some(sheet) = self.sheets.get_mut(&addr.sheet) {
                    sheet.cells.set(addr.row, addr.col, after.clone());
                }
            }
            Change::Sheet { id, after, .. } => match after {
                Some(rec) => {
                    let sheet = self
                        .sheets
                        .entry(*id)
                        .or_insert_with(|| Sheet::new(*id, "", 0));
                    sheet.name.clone_from(&rec.name);
                    sheet.position = rec.position;
                }
                None => {
                    self.sheets.remove(id);
                }
            },
            Change::Layout {
                sheet,
                axis,
                idx,
                after,
                ..
            } => {
                if let Some(sheet) = self.sheets.get_mut(sheet) {
                    let map = sheet.layout.axis_mut(*axis);
                    match after {
                        Some(size) => map.insert(*idx, *size),
                        None => map.remove(idx),
                    };
                }
            }
            Change::Chart { id, after, .. } => match after {
                Some(rec) => {
                    self.charts.insert(*id, rec.clone());
                }
                None => {
                    self.charts.remove(id);
                }
            },
            Change::Structure(op) => self.apply_structure(*op),
        }
    }

    pub fn apply_changes(&mut self, changes: &[Change]) {
        for c in changes {
            self.apply_change(c);
        }
    }

    /// Shifts cells, layout and chart anchors. For deletions the deleted span
    /// is expected to be already empty (the store records those clears as
    /// separate changes first); anything left there is dropped.
    fn apply_structure(&mut self, op: StructOp) {
        let Some(sheet) = self.sheets.get_mut(&op.sheet()) else {
            return;
        };
        let axis = op.axis();
        let cells = sheet.cells.take_all();
        for ((r, c), cell) in cells {
            let moved = match axis {
                Axis::Row => op.map_index(r).map(|r| (r, c)),
                Axis::Col => op.map_index(c).map(|c| (r, c)),
            };
            if let Some((r, c)) = moved {
                sheet.cells.set(r, c, cell);
            }
        }
        let map = sheet.layout.axis_mut(axis);
        *map = std::mem::take(map)
            .into_iter()
            .filter_map(|(i, s)| op.map_index(i).map(|i| (i, s)))
            .collect();
        for chart in self.charts.values_mut() {
            if chart.sheet != op.sheet() {
                continue;
            }
            let idx = match axis {
                Axis::Row => &mut chart.row,
                Axis::Col => &mut chart.col,
            };
            if let Some(i) = op.map_index(*idx) {
                *idx = i;
            }
        }
    }
}
