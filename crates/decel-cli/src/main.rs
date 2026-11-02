// SPDX-License-Identifier: MIT OR Apache-2.0

//! Development CLI for decel.
//!
//! - `decel-cli eval file.decel [--sheet NAME] [--dump csv|values]` opens a
//!   workbook read-only, recalculates everything and prints one sheet as CSV:
//!   displayed text (`csv`, the default) or final values in edit form
//!   (`values`).
//! - `decel-cli demo out.decel` writes the DESIGN §4.7/§4.8 examples.
//! - `decel-cli gen-big out.decel ROWS COLS` writes a large numeric sheet for
//!   scrolling benchmarks.

use std::path::PathBuf;

mod demo;

use anyhow::{Context, Result, bail};
use decel_engine::{Engine, Limits};
use decel_model::{CellAddr, CellStorage};
use decel_store::Store;

const USAGE: &str = "usage: decel-cli eval <file.decel> [--sheet NAME] [--dump csv|values]
       decel-cli demo <out.decel>
       decel-cli gen-big <out.decel> <rows> <cols>";

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
    match args.next().as_deref() {
        Some("eval") => eval(args),
        Some("demo") => demo::write(&PathBuf::from(args.next().context(USAGE)?)),
        Some("gen-big") => {
            let path = PathBuf::from(args.next().context(USAGE)?);
            let rows: u32 = args.next().context(USAGE)?.parse()?;
            let cols: u32 = args.next().context(USAGE)?.parse()?;
            demo::big(&path, rows, cols)
        }
        _ => bail!(USAGE),
    }
}

fn eval(mut args: impl Iterator<Item = String>) -> Result<()> {
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
