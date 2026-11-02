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
