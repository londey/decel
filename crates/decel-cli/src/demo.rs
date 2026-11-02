// SPDX-License-Identifier: MIT OR Apache-2.0

//! Example and benchmark workbooks.

use std::path::Path;

use anyhow::Result;
use decel_model::{Cell, CellAddr, MetaSource, Op, SheetId, TxnInput, Value};
use decel_store::Store;

pub fn latitude() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"latitude\"")
        .with("unit", "\"deg\"")
        .with("display", "return fmt.dms(v, \"lat\", 1)")
        .with("edit", "return string.format(\"%.6f\", v)")
        .with(
            "parse",
            "local n = tonumber(text) or parse.dms(text)\nif not n then return nil, \"Enter decimal degrees or D°M'S\\\" N/S\" end\nreturn n",
        )
        .with(
            "validate",
            "if type(v) ~= \"number\" or v < -90 or v > 90 then return false, \"Latitude must be within ±90°\" end\nreturn true",
        )
        .with("tooltip", "return string.format(\"%.6f° — %s\", v, fmt.dms(v, \"lat\", 2))")
        .with("edit_hint", "return \"Decimal degrees (−35.3075) or 35°18'27\\\"S\"")
}

pub fn longitude() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"longitude\"")
        .with("unit", "\"deg\"")
        .with("display", "return fmt.dms(v, \"lon\", 1)")
        .with("edit", "return string.format(\"%.6f\", v)")
        .with("parse", "return tonumber(text) or parse.dms(text)")
        .with(
            "validate",
            "if type(v) ~= \"number\" or v < -180 or v > 180 then return false, \"Longitude must be within ±180°\" end\nreturn true",
        )
}

pub fn running_total() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"running total\"")
        .with(
            "value",
            "local prev = c:up()\nlocal base = (prev.row >= 1 and prev.kind == self.kind) and prev.value or 0\nreturn base + c:left().value",
        )
        .with("style", "{ align = \"right\", mono = true }")
}

fn s(v: &str) -> Cell {
    Cell::value(Value::str(v))
}

pub fn write(path: &Path) -> Result<()> {
    let mut store = Store::create(path)?;
    let sh = SheetId(1);
    let a = |r: u32, c: u32| CellAddr::new(sh, r, c);
    let (lat, lon, total) = (latitude(), longitude(), running_total());
    let mut cells = vec![
        (a(0, 0), s("Site")),
        (a(0, 1), s("Latitude")),
        (a(0, 2), s("Longitude")),
        (a(0, 4), s("Amount")),
        (a(0, 5), s("Running total")),
    ];
    let sites = [
        ("Alpha", -35.3075, 149.1244),
        ("Bravo", -33.8688, 151.2093),
        ("Charlie", -37.8136, 144.9631),
        ("Delta", -27.4698, 153.0251),
    ];
    for (i, (name, la, lo)) in sites.iter().enumerate() {
        let r = i as u32 + 1;
        cells.push((a(r, 0), s(name)));
        cells.push((a(r, 1), Cell::value(Value::Num(*la)).with_meta(lat.hash())));
        cells.push((a(r, 2), Cell::value(Value::Num(*lo)).with_meta(lon.hash())));
    }
    for r in 1..=20u32 {
        cells.push((a(r, 4), Cell::value(Value::Int(i64::from(r) * 10))));
        cells.push((a(r, 5), Cell::EMPTY.with_meta(total.hash())));
    }
    cells.push((
        a(22, 0),
        s("This text is long enough to spill across the empty cells to its right"),
    ));
    store.apply(
        TxnInput::new("Demo workbook")
            .op(Op::SetCells(cells))
            .meta(lat)
            .meta(lon)
            .meta(total),
    )?;
    Ok(())
}

/// `rows × cols` of numbers, written in chunks.
pub fn big(path: &Path, rows: u32, cols: u32) -> Result<()> {
    let mut store = Store::create(path)?;
    let sh = SheetId(1);
    let chunk = 200_000 / cols.max(1);
    let mut r0 = 0;
    while r0 < rows {
        let r1 = (r0 + chunk).min(rows);
        let mut cells = Vec::with_capacity(((r1 - r0) * cols) as usize);
        for r in r0..r1 {
            for c in 0..cols {
                let v = f64::from(r) * 0.5 + f64::from(c);
                cells.push((CellAddr::new(sh, r, c), Cell::value(Value::Num(v))));
            }
        }
        store.apply(TxnInput::new(format!("rows {r0}..{r1}")).op(Op::SetCells(cells)))?;
        store.checkpoint()?;
        r0 = r1;
    }
    Ok(())
}
