// SPDX-License-Identifier: MIT OR Apache-2.0

//! Fill operations (DESIGN §5.2) as pure functions over a workbook.

use decel_model::{Axis, Cell, CellAddr, Rect, SheetId, Workbook};

/// What a fill copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillWhat {
    /// Metatables only; raw values in the destination are kept.
    Meta,
    /// Metatables and raw values.
    MetaAndValues,
}

/// Fills `dst` by repeating the pattern in `src` (fill drag). Rows and
/// columns of `dst` map onto `src` modulo its size, so dragging in any
/// direction repeats the selection. Cells of `dst` inside `src` are skipped.
pub fn fill_pattern(
    wb: &Workbook,
    sheet: SheetId,
    src: Rect,
    dst: Rect,
    what: FillWhat,
) -> Vec<(CellAddr, Cell)> {
    let h = i64::from(src.r2 - src.r1) + 1;
    let w = i64::from(src.c2 - src.c1) + 1;
    let mut out = Vec::new();
    for (r, c) in dst.iter() {
        if src.contains(r, c) {
            continue;
        }
        let sr = i64::from(src.r1) + (i64::from(r) - i64::from(src.r1)).rem_euclid(h);
        let sc = i64::from(src.c1) + (i64::from(c) - i64::from(src.c1)).rem_euclid(w);
        let from = CellAddr::new(sheet, sr as u32, sc as u32);
        let to = CellAddr::new(sheet, r, c);
        let s = wb.cell(from).cloned().unwrap_or_default();
        let d = wb.cell(to).cloned().unwrap_or_default();
        let cell = match what {
            FillWhat::Meta => Cell {
                raw: d.raw,
                meta: s.meta,
            },
            FillWhat::MetaAndValues => s,
        };
        out.push((to, cell));
    }
    out
}

/// Fill down (`Ctrl+D`) or right (`Ctrl+R`): the first row/column of `sel`
/// is copied over the rest of it.
pub fn fill_from_edge(
    wb: &Workbook,
    sheet: SheetId,
    sel: Rect,
    axis: Axis,
    what: FillWhat,
) -> Vec<(CellAddr, Cell)> {
    let src = match axis {
        Axis::Row => Rect::new(sel.r1, sel.c1, sel.r1, sel.c2),
        Axis::Col => Rect::new(sel.r1, sel.c1, sel.r2, sel.c1),
    };
    fill_pattern(wb, sheet, src, sel, what)
}

/// The adjacent run for auto-fill (DESIGN §5.2): starting from each
/// immediate neighbour of `addr` along `axis` (above/below for
/// [`Axis::Row`], left/right for [`Axis::Col`]), the maximal contiguous
/// block of non-empty cells that share that neighbour's hash ("no
/// metatable" counts as a hash). `addr` itself is not included.
pub fn auto_fill_run(wb: &Workbook, addr: CellAddr, axis: Axis) -> Vec<CellAddr> {
    let step = |a: CellAddr, d: i64| match axis {
        Axis::Row => a.offset(d, 0),
        Axis::Col => a.offset(0, d),
    };
    let mut out = Vec::new();
    for dir in [-1i64, 1] {
        let Some(first) = step(addr, dir) else {
            continue;
        };
        let Some(neighbour) = wb.cell(first).filter(|c| !c.is_empty()) else {
            continue;
        };
        let hash = neighbour.meta;
        let mut cur = Some(first);
        let mut run = Vec::new();
        while let Some(a) = cur {
            match wb.cell(a) {
                Some(c) if !c.is_empty() && c.meta == hash => run.push(a),
                _ => break,
            }
            cur = step(a, dir);
        }
        if dir < 0 {
            run.reverse();
        }
        out.extend(run);
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use decel_model::{CellStorage, MetaSource, Sheet, Value};

    const S: SheetId = SheetId(1);

    /// Builds a column from a compact description, top to bottom:
    /// `.` empty, `v` value without meta, `A`/`B` meta only, `a`/`b` value
    /// with meta. Returns the workbook and the hashes for A and B.
    fn column(spec: &str) -> (Workbook, MetaSource, MetaSource) {
        let a = MetaSource::default().with("kind", "\"a\"");
        let b = MetaSource::default().with("kind", "\"b\"");
        let mut wb = Workbook::default();
        let mut sheet = Sheet::new(S, "S", 0);
        for (r, ch) in spec.chars().enumerate() {
            let cell = match ch {
                '.' => continue,
                'v' => Cell::value(Value::Int(1)),
                'A' => Cell::EMPTY.with_meta(a.hash()),
                'B' => Cell::EMPTY.with_meta(b.hash()),
                'a' => Cell::value(Value::Int(1)).with_meta(a.hash()),
                'b' => Cell::value(Value::Int(1)).with_meta(b.hash()),
                other => panic!("bad spec char {other}"),
            };
            sheet.cells.set(r as u32, 0, cell);
        }
        wb.sheets.insert(S, sheet);
        (wb, a, b)
    }

    fn rows(run: &[CellAddr]) -> Vec<u32> {
        run.iter().map(|a| a.row).collect()
    }

    #[test]
    fn auto_fill_runs() {
        // (column, selected row, expected run rows)
        let cases: &[(&str, u32, &[u32])] = &[
            // A run of "no meta" values below the selection, stopped by a blank.
            ("Avvv.v", 0, &[1, 2, 3]),
            // Stopped by a hash change.
            ("Avvbbv", 0, &[1, 2]),
            // Selection in the middle: both directions, each with its own hash.
            ("bbAvv.", 2, &[0, 1, 3, 4]),
            // Selection at the end of an already-filled run, values below.
            ("aaaAvv", 3, &[0, 1, 2, 4, 5]),
            // Blank immediately below: nothing that way.
            ("A.vv", 0, &[]),
            // Selection at the very top: nothing above.
            ("Av", 0, &[1]),
            // Meta-only cells count as non-empty.
            ("ABBB.", 0, &[1, 2, 3]),
        ];
        for (spec, sel, want) in cases {
            let (wb, _, _) = column(spec);
            let run = auto_fill_run(&wb, CellAddr::new(S, *sel, 0), Axis::Row);
            assert_eq!(rows(&run), *want, "{spec} @ {sel}");
        }
    }

    #[test]
    fn auto_fill_along_a_row() {
        let (mut wb, a, _) = column("");
        let sheet = wb.sheets.get_mut(&S).unwrap();
        sheet.cells.set(0, 0, Cell::EMPTY.with_meta(a.hash()));
        for c in 1..4 {
            sheet.cells.set(0, c, Cell::value(Value::Int(1)));
        }
        let run = auto_fill_run(&wb, CellAddr::new(S, 0, 0), Axis::Col);
        assert_eq!(run.iter().map(|a| a.col).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn fill_pattern_repeats_and_keeps_values() {
        let (wb, a, b) = column("ab");
        let out = fill_pattern(
            &wb,
            S,
            Rect::new(0, 0, 1, 0),
            Rect::new(0, 0, 5, 0),
            FillWhat::Meta,
        );
        let metas: Vec<_> = out.iter().map(|(_, c)| c.meta).collect();
        assert_eq!(
            metas,
            [
                Some(a.hash()),
                Some(b.hash()),
                Some(a.hash()),
                Some(b.hash())
            ]
        );
        assert!(out.iter().all(|(_, c)| c.raw.is_none()));
        let out = fill_pattern(
            &wb,
            S,
            Rect::new(0, 0, 1, 0),
            Rect::new(2, 0, 2, 0),
            FillWhat::MetaAndValues,
        );
        assert_eq!(out[0].1.raw, Some(Value::Int(1)));
    }

    #[test]
    fn fill_upwards_repeats_pattern() {
        let (wb, a, b) = column("..ab");
        let out = fill_pattern(
            &wb,
            S,
            Rect::new(2, 0, 3, 0),
            Rect::new(0, 0, 3, 0),
            FillWhat::Meta,
        );
        assert_eq!(out[0].1.meta, Some(a.hash()));
        assert_eq!(out[1].1.meta, Some(b.hash()));
    }

    #[test]
    fn fill_down_from_top_row() {
        let (wb, a, _) = column("A..");
        let out = fill_from_edge(&wb, S, Rect::new(0, 0, 2, 0), Axis::Row, FillWhat::Meta);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(_, c)| c.meta == Some(a.hash())));
    }
}
