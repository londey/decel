// SPDX-License-Identifier: MIT OR Apache-2.0

//! Keyboard navigation helpers that need the whole sheet.

use decel_model::{CellAddr, CellStorage, Workbook};

fn filled(wb: &Workbook, a: CellAddr) -> bool {
    wb.cell(a).is_some_and(|c| !c.is_empty())
}

/// Ctrl+arrow: from a filled cell whose neighbour is filled, go to the end
/// of that run; otherwise go to the next filled cell. With nothing further,
/// stop at the sheet edge (row/col 0) or the last used row/column.
pub fn data_edge(wb: &Workbook, from: CellAddr, dr: i64, dc: i64) -> CellAddr {
    let Some(sheet) = wb.sheet(from.sheet) else {
        return from;
    };
    let limit_r = i64::from(sheet.cells.last_row().unwrap_or(0).max(from.row));
    let limit_c = i64::from(sheet.cells.last_col().unwrap_or(0).max(from.col));
    let in_bounds = |a: CellAddr| i64::from(a.row) <= limit_r && i64::from(a.col) <= limit_c;
    let step = |a: CellAddr| a.offset(dr, dc).filter(|n| in_bounds(*n));
    let Some(next) = step(from) else {
        return from;
    };
    if filled(wb, from) && filled(wb, next) {
        let mut cur = next;
        while let Some(n) = step(cur) {
            if !filled(wb, n) {
                break;
            }
            cur = n;
        }
        return cur;
    }
    let mut cur = next;
    loop {
        if filled(wb, cur) {
            return cur;
        }
        match step(cur) {
            Some(n) => cur = n,
            None => return cur,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decel_model::{Cell, Sheet, SheetId, Value};

    #[test]
    fn jumps() {
        let s = SheetId(1);
        let mut wb = Workbook::default();
        let mut sheet = Sheet::new(s, "S", 0);
        for r in [2, 3, 4, 8] {
            sheet.cells.set(r, 0, Cell::value(Value::Int(1)));
        }
        wb.sheets.insert(s, sheet);
        let a = |r| CellAddr::new(s, r, 0);
        assert_eq!(data_edge(&wb, a(0), 1, 0), a(2)); // to next filled
        assert_eq!(data_edge(&wb, a(2), 1, 0), a(4)); // end of run
        assert_eq!(data_edge(&wb, a(4), 1, 0), a(8)); // next run
        assert_eq!(data_edge(&wb, a(8), 1, 0), a(8)); // nothing further
        assert_eq!(data_edge(&wb, a(8), -1, 0), a(4));
        assert_eq!(data_edge(&wb, a(2), -1, 0), a(0)); // to edge
        assert_eq!(data_edge(&wb, a(0), 0, -1), a(0));
    }
}
