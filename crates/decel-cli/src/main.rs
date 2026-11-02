// SPDX-License-Identifier: MIT OR Apache-2.0

//! `decel-cli eval file.decel [--sheet NAME] [--dump csv|values]`
//!
//! Opens a workbook read-only, recalculates everything and prints one sheet
//! as CSV: displayed text (`csv`, the default) or final values in edit form
//! (`values`). For end-to-end tests and debugging.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use decel_engine::{Engine, Limits};
use decel_model::{CellAddr, CellStorage};
use decel_store::Store;

const USAGE: &str = "usage: decel-cli eval <file.decel> [--sheet NAME] [--dump csv|values]";

enum Dump {
    Display,
    Values,
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("eval") {
        bail!(USAGE);
    }
    let path = PathBuf::from(args.next().context(USAGE)?);
    let mut sheet_name = None;
    let mut dump = Dump::Display;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sheet" => sheet_name = Some(args.next().context(USAGE)?),
            "--dump" => {
                dump = match args.next().as_deref() {
                    Some("csv") => Dump::Display,
                    Some("values") => Dump::Values,
                    _ => bail!(USAGE),
                }
            }
            _ => bail!(USAGE),
        }
    }

    let store =
        Store::open_read_only(&path).with_context(|| format!("opening {}", path.display()))?;
    let wb = store.load()?;
    let sheet = match &sheet_name {
        Some(n) => wb
            .sheet_by_name(n)
            .with_context(|| format!("no sheet named {n:?}"))?,
        None => *wb
            .sheets_ordered()
            .first()
            .context("workbook has no sheets")?,
    };
    let (id, rows, cols) = (
        sheet.id,
        sheet.cells.last_row().map_or(0, |r| r + 1),
        sheet.cells.last_col().map_or(0, |c| c + 1),
    );
    let mut engine = Engine::new(wb, Limits::default())?;
    engine.recalc_all();
    for r in 0..rows {
        let line: Vec<String> = (0..cols)
            .map(|c| {
                let a = CellAddr::new(id, r, c);
                let text = match dump {
                    Dump::Display => engine.display(a).text,
                    Dump::Values => engine.final_value(a).edit_text(),
                };
                csv_field(&text)
            })
            .collect();
        println!("{}", line.join(","));
    }
    Ok(())
}
