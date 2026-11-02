// SPDX-License-Identifier: MIT OR Apache-2.0

//! A small DSL for building workbooks and asserting values headlessly,
//! without SQLite. Rows and columns are **1-based**, as in the UI.
//!
//! ```
//! use decel_engine::testkit::{Wb, meta};
//! let mut wb = Wb::new();
//! wb.set(1, 1, 10).set(2, 1, 5);
//! wb.meta(1, 2, &meta().with("value", "return c:left().value * 2"));
//! assert_eq!(wb.value(1, 2), 20.into());
//! ```

use decel_model::{
    Cell, CellAddr, Change, MetaSource, Sheet, SheetId, SheetRecord, Value, Workbook,
};

use crate::{DisplayCell, EditRejected, Engine, Limits};

/// An empty metatable to build on with [`MetaSource::with`].
pub fn meta() -> MetaSource {
    MetaSource::default()
}

pub struct Wb {
    pub engine: Engine,
    pub sheet: SheetId,
}

impl Default for Wb {
    fn default() -> Self {
        Self::new()
    }
}

fn addr(sheet: SheetId, row: u32, col: u32) -> CellAddr {
    assert!(row >= 1 && col >= 1, "testkit rows/cols are 1-based");
    CellAddr::new(sheet, row - 1, col - 1)
}

impl Wb {
    pub fn new() -> Self {
        Self::with_limits(Limits::default())
    }

    pub fn with_limits(limits: Limits) -> Self {
        let mut wb = Workbook::default();
        let id = SheetId(1);
        wb.sheets.insert(id, Sheet::new(id, "Sheet1", 0));
        Self {
            engine: Engine::new(wb, limits).unwrap_or_else(|e| panic!("engine: {e}")),
            sheet: id,
        }
    }

    pub fn addr(&self, row: u32, col: u32) -> CellAddr {
        addr(self.sheet, row, col)
    }

    /// Sets cells, as one batch of changes.
    pub fn apply_cells(&mut self, cells: Vec<(CellAddr, Cell)>) -> &mut Self {
        let changes: Vec<Change> = cells
            .into_iter()
            .map(|(a, after)| Change::Cell {
                addr: a,
                before: self.engine.workbook().cell(a).cloned().unwrap_or_default(),
                after,
            })
            .collect();
        self.engine.apply(&changes);
        self
    }

    fn current(&self, a: CellAddr) -> Cell {
        self.engine.workbook().cell(a).cloned().unwrap_or_default()
    }

    /// Sets a raw value, keeping the cell's metatable.
    pub fn set(&mut self, row: u32, col: u32, v: impl Into<Value>) -> &mut Self {
        let a = self.addr(row, col);
        let cell = Cell {
            raw: Some(v.into()),
            ..self.current(a)
        };
        self.apply_cells(vec![(a, cell)])
    }

    pub fn clear(&mut self, row: u32, col: u32) -> &mut Self {
        let a = self.addr(row, col);
        self.apply_cells(vec![(a, Cell::EMPTY)])
    }

    /// Sets a cell's metatable, keeping its raw value.
    pub fn meta(&mut self, row: u32, col: u32, src: &MetaSource) -> &mut Self {
        self.fill_meta(row, row, col, src)
    }

    /// Sets the same metatable on rows `r1..=r2` of `col` (shared, as by a fill).
    pub fn fill_meta(&mut self, r1: u32, r2: u32, col: u32, src: &MetaSource) -> &mut Self {
        let hash = self
            .engine
            .add_meta(src)
            .unwrap_or_else(|e| panic!("add_meta: {e}"));
        let cells = (r1..=r2)
            .map(|r| {
                let a = self.addr(r, col);
                (
                    a,
                    Cell {
                        meta: Some(hash),
                        ..self.current(a)
                    },
                )
            })
            .collect();
        self.apply_cells(cells)
    }

    pub fn remove_meta(&mut self, row: u32, col: u32) -> &mut Self {
        let a = self.addr(row, col);
        let cell = Cell {
            meta: None,
            ..self.current(a)
        };
        self.apply_cells(vec![(a, cell)])
    }

    /// Types text into a cell: parse, validate, then commit.
    pub fn type_text(&mut self, row: u32, col: u32, text: &str) -> Result<(), EditRejected> {
        let a = self.addr(row, col);
        let cell = self.engine.enter_text(a, text)?;
        self.apply_cells(vec![(a, cell)]);
        Ok(())
    }

    /// Adds a sheet and returns its id.
    pub fn add_sheet(&mut self, name: &str) -> SheetId {
        let id = SheetId(
            self.engine
                .workbook()
                .sheets
                .keys()
                .map(|s| s.0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let position = self.engine.workbook().sheets.len() as u32;
        self.engine.apply(&[Change::Sheet {
            id,
            before: None,
            after: Some(SheetRecord {
                name: name.into(),
                position,
            }),
        }]);
        id
    }

    pub fn rename_sheet(&mut self, id: SheetId, name: &str) {
        let before = self
            .engine
            .workbook()
            .sheet(id)
            .map(Sheet::record)
            .unwrap_or_else(|| panic!("no sheet {id}"));
        let after = SheetRecord {
            name: name.into(),
            ..before.clone()
        };
        self.engine.apply(&[Change::Sheet {
            id,
            before: Some(before),
            after: Some(after),
        }]);
    }

    pub fn recalc(&mut self) -> &mut Self {
        self.engine.recalc_all();
        self
    }

    pub fn value(&mut self, row: u32, col: u32) -> Value {
        self.recalc();
        let a = self.addr(row, col);
        self.engine.final_value(a)
    }

    pub fn display_cell(&mut self, row: u32, col: u32) -> DisplayCell {
        self.recalc();
        let a = self.addr(row, col);
        self.engine.display(a)
    }

    pub fn display(&mut self, row: u32, col: u32) -> String {
        self.display_cell(row, col).text
    }

    pub fn tooltip(&mut self, row: u32, col: u32) -> Option<String> {
        self.recalc();
        let a = self.addr(row, col);
        self.engine.tooltip(a)
    }

    pub fn edit_text(&mut self, row: u32, col: u32) -> String {
        let a = self.addr(row, col);
        self.engine.edit_text(a)
    }

    pub fn edit_hint(&mut self, row: u32, col: u32) -> Option<String> {
        let a = self.addr(row, col);
        self.engine.edit_hint(a)
    }
}
