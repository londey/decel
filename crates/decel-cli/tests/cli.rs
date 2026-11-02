// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

use decel_model::{Cell, CellAddr, MetaSource, Op, SheetId, TxnInput, Value};
use decel_store::Store;

#[test]
fn eval_dumps_display_and_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e2e.decel");
    {
        let mut store = Store::create(&path).unwrap();
        let m = MetaSource::default()
            .with("value", "return c:left().value * 2")
            .with("display", "return 'x' .. v");
        let h = m.hash();
        let s = SheetId(1);
        let cells = vec![
            (CellAddr::new(s, 0, 0), Cell::value(Value::Int(3))),
            (CellAddr::new(s, 0, 1), Cell::EMPTY.with_meta(h)),
            (CellAddr::new(s, 1, 0), Cell::value(Value::str("a, \"b\""))),
        ];
        store
            .apply(TxnInput::new("t").op(Op::SetCells(cells)).meta(m))
            .unwrap();
    }
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_decel-cli"))
            .arg("eval")
            .arg(&path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(run(&[]), "3,x6\n\"a, \"\"b\"\"\",\n");
    assert_eq!(
        run(&["--dump", "values", "--sheet", "Sheet1"]),
        "3,6\n\"a, \"\"b\"\"\",\n"
    );
}

/// Runs `decel-cli` with `args` and returns stdout, failing on a non-zero exit.
fn cli(args: &[&std::ffi::OsStr]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_decel-cli"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn examples_evaluate_without_errors() {
    let dir = tempfile::tempdir().unwrap();
    cli(&["examples".as_ref(), dir.path().as_os_str()]);
    let sheets = [
        ("survey.decel", "Survey"),
        ("budget.decel", "Budget"),
        ("budget.decel", "Rates"),
        ("engineering.decel", "Beam"),
    ];
    let mut dumps = std::collections::HashMap::new();
    for (file, sheet) in sheets {
        let path = dir.path().join(file);
        let text = cli(&[
            "eval".as_ref(),
            path.as_os_str(),
            "--sheet".as_ref(),
            sheet.as_ref(),
        ]);
        println!("== {file} / {sheet}\n{text}");
        for code in ["#LUA", "#CYCLE", "#TIMEOUT", "#MEMORY", "#REF", "#TYPE"] {
            assert!(!text.contains(code), "{file}/{sheet} shows {code}:\n{text}");
        }
        dumps.insert(sheet, text);
    }

    // Spot-check that the Lua really ran.
    let survey = &dumps["Survey"];
    assert!(survey.contains("603 m,0 m,14 Mar 2026"), "{survey}");
    assert!(survey.contains("Highest,,,888 m"), "{survey}");
    let budget = &dumps["Budget"];
    assert!(budget.contains("\"A$6,200.00\""), "{budget}");
    assert!(budget.contains("Total,,\"A$2,228.15\""), "{budget}");
    assert!(budget.contains("4,030.00 USD"), "{budget}");
    let beam = &dumps["Beam"];
    assert!(beam.contains("Force,750 kN"), "{beam}");
    assert!(beam.contains("Stress,150 MPa"), "{beam}");
    assert!(beam.contains("Utilisation,60.0%"), "{beam}");
}

/// Opens a generated example in a headless engine and recalculates it.
fn open_example(dir: &std::path::Path, file: &str) -> (decel_engine::Engine, Vec<SheetId>) {
    let store = Store::open_read_only(&dir.join(file)).unwrap();
    let wb = store.load().unwrap();
    let sheets = wb.sheets_ordered().iter().map(|s| s.id).collect();
    let mut engine = decel_engine::Engine::new(wb, decel_engine::Limits::default()).unwrap();
    engine.recalc_all();
    (engine, sheets)
}

#[test]
fn example_hooks_parse_validate_and_warn() {
    let dir = tempfile::tempdir().unwrap();
    cli(&["examples".as_ref(), dir.path().as_os_str()]);

    // Survey: latitude parse/validate, elevation parse, date parse, tooltip.
    let (mut e, sheets) = open_example(dir.path(), "survey.decel");
    let a = |r, c| CellAddr::new(sheets[0], r, c);
    assert!(e.enter_text(a(1, 1), "35 18 27 S").is_ok());
    let err = e.enter_text(a(1, 1), "95").unwrap_err();
    assert_eq!(err.message, "Latitude must be within ±90°");
    assert_eq!(
        e.enter_text(a(1, 3), "612.5 m").unwrap().raw,
        Some(Value::Num(612.5))
    );
    assert!(e.enter_text(a(1, 3), "tall").is_err());
    assert_eq!(
        e.enter_text(a(1, 5), "14 Mar 2026").unwrap().raw,
        Some(Value::Num(20526.0))
    );
    assert_eq!(e.edit_text(a(1, 5)), "2026-03-14");
    let tip = e.tooltip(a(2, 4)).unwrap();
    assert!(tip.ends_with("m from Capital Hill"), "{tip}");
    assert!(e.warning(a(2, 4)).is_none());

    // Budget: currency parse, cross-sheet tooltip.
    let (mut e, sheets) = open_example(dir.path(), "budget.decel");
    let a = |r, c| CellAddr::new(sheets[0], r, c);
    assert_eq!(
        e.enter_text(a(2, 2), "-A$2,200.00").unwrap().raw,
        Some(Value::Num(-2200.0))
    );
    assert_eq!(
        e.enter_text(a(2, 2), "(15.50)").unwrap().raw,
        Some(Value::Num(-15.5))
    );
    assert!(e.enter_text(a(2, 2), "lots").is_err());
    assert_eq!(e.edit_text(a(2, 2)), "-2150.00");
    let tip = e.tooltip(a(1, 6)).unwrap();
    assert_eq!(tip, "1 AUD = 0.6 EUR (from the Rates sheet)");

    // Engineering: only the deliberate mixed-unit sum warns.
    let (mut e, sheets) = open_example(dir.path(), "engineering.decel");
    let a = |r, c| CellAddr::new(sheets[0], r, c);
    for r in 1..=10 {
        assert!(
            e.warning(a(r, 1)).is_none(),
            "row {r}: {:?}",
            e.warning(a(r, 1))
        );
    }
    assert_eq!(e.warning(a(15, 1)).as_deref(), Some("Mixed units: N, m"));
    assert!(e.display(a(15, 1)).has_marker());
    assert_eq!(
        e.enter_text(a(1, 1), "480000 N").unwrap().raw,
        Some(Value::Num(480000.0))
    );
}
