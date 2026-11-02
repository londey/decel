// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

use decel_model::{
    Axis, Cell, CellAddr, CellStorage, ChartId, ChartRecord, MetaSource, Op, SheetId, StructOp,
    TxnInput, Value,
};
use decel_store::{Store, StoreError, TxnKind};

const S1: SheetId = SheetId(1);

fn a(row: u32, col: u32) -> CellAddr {
    CellAddr::new(S1, row, col)
}

fn int(i: i64) -> Cell {
    Cell::value(Value::Int(i))
}

fn set(label: &str, cells: Vec<(CellAddr, Cell)>) -> TxnInput {
    TxnInput::new(label).op(Op::SetCells(cells))
}

#[test]
fn new_workbook_has_sheet1() {
    let store = Store::in_memory().unwrap();
    let wb = store.load().unwrap();
    let sheets = wb.sheets_ordered();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].name, "Sheet1");
    assert_eq!(sheets[0].id, S1);
    assert!(!store.can_undo());
}

#[test]
fn set_load_undo_redo() {
    let mut s = Store::in_memory().unwrap();
    s.apply(set("one", vec![(a(0, 0), int(1))])).unwrap();
    s.apply(set(
        "two",
        vec![(a(0, 0), int(2)), (a(1, 0), Cell::value(Value::str("x")))],
    ))
    .unwrap();
    assert_eq!(s.load().unwrap().cell(a(0, 0)), Some(&int(2)));
    assert_eq!(s.undo_label().unwrap().as_deref(), Some("two"));

    let undone = s.undo().unwrap().unwrap();
    assert_eq!(undone.txn.kind, TxnKind::Undo);
    assert_eq!(undone.txn.label, "Undo two");
    let wb = s.load().unwrap();
    assert_eq!(wb.cell(a(0, 0)), Some(&int(1)));
    assert_eq!(wb.cell(a(1, 0)), None);
    assert_eq!(s.redo_label().unwrap().as_deref(), Some("two"));

    s.redo().unwrap().unwrap();
    assert_eq!(s.load().unwrap().cell(a(0, 0)), Some(&int(2)));
    // Undo of a redo, labelled with the original action.
    assert_eq!(s.undo_label().unwrap().as_deref(), Some("two"));
    assert_eq!(s.undo().unwrap().unwrap().txn.label, "Undo two");
    s.undo().unwrap().unwrap();
    assert!(s.load().unwrap().sheet(S1).unwrap().cells.is_empty());
    assert!(s.undo().unwrap().is_none());
}

#[test]
fn new_edit_clears_redo() {
    let mut s = Store::in_memory().unwrap();
    s.apply(set("one", vec![(a(0, 0), int(1))])).unwrap();
    s.undo().unwrap();
    assert!(s.can_redo());
    s.apply(set("two", vec![(a(0, 1), int(2))])).unwrap();
    assert!(!s.can_redo());
}

#[test]
fn noop_txn_is_not_recorded() {
    let mut s = Store::in_memory().unwrap();
    s.apply(set("one", vec![(a(0, 0), int(1))])).unwrap();
    assert!(
        s.apply(set("same", vec![(a(0, 0), int(1))]))
            .unwrap()
            .is_none()
    );
    assert!(
        s.apply(set("empty", vec![(a(5, 5), Cell::EMPTY)]))
            .unwrap()
            .is_none()
    );
    assert_eq!(s.txns().unwrap().len(), 1);
}

#[test]
fn repeated_cell_in_one_txn_merges() {
    let mut s = Store::in_memory().unwrap();
    let c = s
        .apply(set("x", vec![(a(0, 0), int(1)), (a(0, 0), int(2))]))
        .unwrap()
        .unwrap();
    assert_eq!(c.changes.len(), 1);
    assert_eq!(s.load().unwrap().cell(a(0, 0)), Some(&int(2)));
    s.undo().unwrap();
    assert_eq!(s.load().unwrap().cell(a(0, 0)), None);
}

#[test]
fn rejects_bad_input_without_committing() {
    let mut s = Store::in_memory().unwrap();
    let unknown = MetaSource::default().with("kind", "\"x\"").hash();
    assert!(matches!(
        s.apply(set("m", vec![(a(0, 0), int(1).with_meta(unknown))])),
        Err(StoreError::UnknownMeta(_))
    ));
    assert!(matches!(
        s.apply(set("s", vec![(CellAddr::new(SheetId(9), 0, 0), int(1))])),
        Err(StoreError::NoSuchSheet(_))
    ));
    assert!(matches!(
        s.apply(set("e", vec![(a(0, 0), Cell::value(Value::Nil))])),
        Err(StoreError::InvalidRaw)
    ));
    assert!(s.txns().unwrap().is_empty());
}

#[test]
fn metatables_are_stored_and_shared() {
    let mut s = Store::in_memory().unwrap();
    let meta = MetaSource::default()
        .with("kind", "\"running total\"")
        .with("value", "return 1");
    let h = meta.hash();
    let cells = (0..5)
        .map(|r| (a(r, 1), Cell::EMPTY.with_meta(h)))
        .collect();
    s.apply(
        TxnInput::new("fill")
            .op(Op::SetCells(cells))
            .meta(meta.clone()),
    )
    .unwrap();
    assert_eq!(s.metatable(&h).unwrap().unwrap().hash(), h);
    assert_eq!(s.share_set(&h).unwrap().len(), 5);
    let wb = s.load().unwrap();
    assert_eq!(wb.meta(&h).unwrap().hash(), h);
    assert_eq!(wb.cell(a(3, 1)).unwrap().meta, Some(h));
}

#[test]
fn sheet_ops_and_undo() {
    let mut s = Store::in_memory().unwrap();
    let c = s
        .apply(TxnInput::new("add").op(Op::AddSheet {
            name: "Inputs".into(),
            position: Some(0),
        }))
        .unwrap()
        .unwrap();
    let wb = s.load().unwrap();
    let names: Vec<_> = wb.sheets_ordered().iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["Inputs", "Sheet1"]);
    assert!(c.changes.len() >= 2);
    let inputs = wb.sheet_by_name("Inputs").unwrap().id;

    assert!(matches!(
        s.apply(TxnInput::new("dup").op(Op::RenameSheet {
            id: S1,
            name: "Inputs".into()
        })),
        Err(StoreError::DuplicateSheetName(_))
    ));

    s.apply(set("v", vec![(CellAddr::new(inputs, 2, 2), int(7))]))
        .unwrap();
    s.apply(TxnInput::new("chart").op(Op::ChartUpsert {
        id: None,
        chart: ChartRecord {
            sheet: inputs,
            row: 0,
            col: 0,
            spec: "{}".into(),
        },
    }))
    .unwrap();
    s.apply(TxnInput::new("del").op(Op::DeleteSheet { id: inputs }))
        .unwrap();
    let wb = s.load().unwrap();
    assert_eq!(wb.sheets.len(), 1);
    assert!(wb.charts.is_empty());
    assert_eq!(wb.sheet(S1).unwrap().position, 0);

    s.undo().unwrap();
    let wb = s.load().unwrap();
    assert_eq!(wb.sheet_by_name("Inputs").unwrap().position, 0);
    assert_eq!(wb.cell(CellAddr::new(inputs, 2, 2)), Some(&int(7)));
    assert_eq!(wb.charts.len(), 1);

    assert!(matches!(
        s.apply(TxnInput::new("del").op(Op::DeleteSheet { id: S1 }))
            .and_then(|_| s.apply(TxnInput::new("del").op(Op::DeleteSheet { id: inputs }))),
        Err(StoreError::LastSheet)
    ));
}

#[test]
fn move_sheet_reorders() {
    let mut s = Store::in_memory().unwrap();
    for n in ["B", "C"] {
        s.apply(TxnInput::new("add").op(Op::AddSheet {
            name: n.into(),
            position: None,
        }))
        .unwrap();
    }
    s.apply(TxnInput::new("mv").op(Op::MoveSheet {
        id: S1,
        position: 2,
    }))
    .unwrap();
    let order = |s: &Store| -> Vec<String> {
        s.load()
            .unwrap()
            .sheets_ordered()
            .iter()
            .map(|s| s.name.clone())
            .collect()
    };
    assert_eq!(order(&s), ["B", "C", "Sheet1"]);
    s.undo().unwrap();
    assert_eq!(order(&s), ["Sheet1", "B", "C"]);
}

#[test]
fn layout_and_charts() {
    let mut s = Store::in_memory().unwrap();
    s.apply(TxnInput::new("w").op(Op::SetLayout {
        sheet: S1,
        axis: Axis::Col,
        idx: 3,
        size: Some(120.0),
    }))
    .unwrap();
    assert_eq!(s.load().unwrap().sheet(S1).unwrap().layout.cols[&3], 120.0);
    assert!(matches!(
        s.apply(TxnInput::new("w").op(Op::SetLayout {
            sheet: S1,
            axis: Axis::Col,
            idx: 3,
            size: Some(f64::NAN),
        })),
        Err(StoreError::InvalidSize)
    ));
    s.undo().unwrap();
    assert!(s.load().unwrap().sheet(S1).unwrap().layout.cols.is_empty());
    assert!(matches!(
        s.apply(TxnInput::new("c").op(Op::ChartDelete { id: ChartId(4) })),
        Err(StoreError::NoSuchChart(_))
    ));
}

#[test]
fn history_and_restore() {
    let mut s = Store::in_memory().unwrap();
    for i in 1..=3 {
        s.apply(set(&format!("edit {i}"), vec![(a(0, 0), int(i))]))
            .unwrap();
    }
    let hist = s.cell_history(a(0, 0)).unwrap();
    assert_eq!(hist.len(), 3);
    assert_eq!(hist[0].after, int(3));
    assert_eq!(hist[2].before, Cell::EMPTY);
    let first = hist[2].txn.id;

    s.restore(a(0, 0), first, a(0, 0)).unwrap().unwrap();
    assert_eq!(s.load().unwrap().cell(a(0, 0)), Some(&int(1)));
    let latest = &s.cell_history(a(0, 0)).unwrap()[0];
    assert_eq!(latest.txn.kind, TxnKind::Restore);
    assert_eq!(latest.txn.target, Some(first));

    // Restore then undo returns to the pre-restore state.
    s.undo().unwrap();
    assert_eq!(s.load().unwrap().cell(a(0, 0)), Some(&int(3)));
}

#[test]
fn history_follows_cell_through_row_inserts() {
    let mut s = Store::in_memory().unwrap();
    s.apply(set("v1", vec![(a(5, 0), int(1))])).unwrap();
    for _ in 0..2 {
        s.apply(TxnInput::new("ins").op(Op::Structure(StructOp::InsertRows {
            sheet: S1,
            at: 2,
            n: 3,
        })))
        .unwrap();
    }
    s.apply(set("v2", vec![(a(11, 0), int(2))])).unwrap();
    let hist = s.cell_history(a(11, 0)).unwrap();
    let labels: Vec<_> = hist.iter().map(|v| v.txn.label.as_str()).collect();
    assert_eq!(labels, ["v2", "v1"]);
    assert_eq!(hist[1].addr, a(5, 0));
}

#[test]
fn delete_rows_and_undo() {
    let mut s = Store::in_memory().unwrap();
    let cells = (0..10).map(|r| (a(r, 0), int(i64::from(r)))).collect();
    s.apply(set("fill", cells)).unwrap();
    s.apply(TxnInput::new("del").op(Op::Structure(StructOp::DeleteRows {
        sheet: S1,
        at: 2,
        n: 3,
    })))
    .unwrap();
    let wb = s.load().unwrap();
    assert_eq!(wb.cell(a(2, 0)), Some(&int(5)));
    assert_eq!(wb.sheet(S1).unwrap().cells.len(), 7);
    s.undo().unwrap();
    let wb = s.load().unwrap();
    for r in 0..10 {
        assert_eq!(wb.cell(a(r, 0)), Some(&int(i64::from(r))));
    }
}

#[test]
fn structure_must_be_alone() {
    let mut s = Store::in_memory().unwrap();
    let txn = TxnInput::new("x")
        .op(Op::Structure(StructOp::InsertRows {
            sheet: S1,
            at: 0,
            n: 1,
        }))
        .op(Op::SetCells(vec![]));
    assert!(matches!(s.apply(txn), Err(StoreError::StructureNotAlone)));
}

#[test]
fn file_persists_undo_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.decel");
    {
        let mut s = Store::create(&path).unwrap();
        s.apply(set("one", vec![(a(0, 0), int(1))])).unwrap();
        s.apply(set("two", vec![(a(0, 0), int(2))])).unwrap();
        s.undo().unwrap();
    }
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.load().unwrap().cell(a(0, 0)), Some(&int(1)));
    assert!(s.can_redo());
    s.undo().unwrap();
    assert_eq!(s.load().unwrap().cell(a(0, 0)), None);
}

#[test]
fn lock_prevents_second_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.decel");
    let mut first = Store::create(&path).unwrap();
    match Store::open(&path) {
        Err(StoreError::Locked { pid, host }) => {
            if cfg!(unix) {
                assert_eq!(pid, Some(std::process::id()));
                assert!(host.is_some());
            }
        }
        other => panic!("expected Locked, got {:?}", other.err()),
    }
    first.apply(set("one", vec![(a(0, 0), int(1))])).unwrap();
    // A read-only open still works and sees committed data.
    let mut ro = Store::open_read_only(&path).unwrap();
    assert!(ro.is_read_only());
    assert_eq!(ro.load().unwrap().cell(a(0, 0)), Some(&int(1)));
    assert!(matches!(
        ro.apply(set("x", vec![(a(0, 0), int(2))])),
        Err(StoreError::ReadOnly)
    ));
    drop(first);
    // Lock released on drop.
    Store::open(&path).unwrap();
}

#[test]
fn rejects_foreign_files() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("t.decel");
    std::fs::write(&text, "this is not sqlite at all, just some text").unwrap();
    assert!(matches!(Store::open(&text), Err(StoreError::NotADecelFile)));

    let other = dir.path().join("o.decel");
    {
        let conn = rusqlite::Connection::open(&other).unwrap();
        conn.execute_batch("CREATE TABLE t(x)").unwrap();
    }
    assert!(matches!(
        Store::open(&other),
        Err(StoreError::NotADecelFile)
    ));
}

#[test]
fn save_as_copies_everything_and_switches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.decel");
    let copy = dir.path().join("b.decel");
    let mut s = Store::create(&path).unwrap();
    s.apply(set("one", vec![(a(0, 0), int(1))])).unwrap();
    s.set_setting("theme", "dark").unwrap();
    s.save_as(&copy).unwrap();
    assert_eq!(s.path(), Some(copy.as_path()));
    assert!(s.can_undo());
    s.apply(set("two", vec![(a(0, 0), int(2))])).unwrap();
    drop(s);
    let orig = Store::open(&path).unwrap();
    assert_eq!(orig.load().unwrap().cell(a(0, 0)), Some(&int(1)));
    let copy = Store::open(&copy).unwrap();
    let wb = copy.load().unwrap();
    assert_eq!(wb.cell(a(0, 0)), Some(&int(2)));
    assert_eq!(wb.settings["theme"], "dark");
}

#[test]
fn nan_and_special_values_round_trip() {
    let mut s = Store::in_memory().unwrap();
    let vals = [
        Value::Num(f64::NAN),
        Value::Num(f64::INFINITY),
        Value::Num(-0.0),
        Value::Bool(false),
        Value::Int(i64::MIN),
        Value::str(""),
        Value::str("multi\nline, \"quoted\""),
    ];
    let cells = vals
        .iter()
        .enumerate()
        .map(|(i, v)| (a(i as u32, 0), Cell::value(v.clone())))
        .collect();
    s.apply(set("v", cells)).unwrap();
    let wb = s.load().unwrap();
    for (i, v) in vals.iter().enumerate() {
        assert_eq!(wb.cell(a(i as u32, 0)).unwrap().raw.as_ref(), Some(v));
    }
}

#[test]
fn history_of_fifty_edits_over_three_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("h.decel");
    let mut n = 0;
    for session in 0..3 {
        let mut s = if session == 0 {
            Store::create(&path).unwrap()
        } else {
            Store::open(&path).unwrap()
        };
        let edits = if session < 2 { 17 } else { 16 };
        for _ in 0..edits {
            n += 1;
            s.apply(set(
                &format!("edit {n}"),
                vec![(a(0, 0), int(n)), (a(1, n as u32 % 3), int(n))],
            ))
            .unwrap();
        }
    }
    let s = Store::open(&path).unwrap();
    let hist = s.cell_history(a(0, 0)).unwrap();
    assert_eq!(hist.len(), 50);
    for (i, v) in hist.iter().enumerate() {
        assert_eq!(v.after, int(50 - i as i64));
        assert_eq!(v.txn.label, format!("edit {}", 50 - i));
    }
    assert!(hist.windows(2).all(|w| w[0].txn.id > w[1].txn.id));
}

#[test]
fn charts_follow_structure_and_undo_exactly() {
    use decel_model::{ChartSpec, RangeRef, Rect, Series};
    let mut s = Store::in_memory().unwrap();
    let spec = ChartSpec {
        series: vec![Series {
            name: "y".into(),
            range: RangeRef {
                sheet: S1,
                rect: Rect::new(2, 1, 9, 1),
            },
            color: None,
        }],
        ..ChartSpec::default()
    };
    let rec = ChartRecord {
        sheet: S1,
        row: 3,
        col: 4,
        spec: spec.to_json(),
    };
    s.apply(TxnInput::new("chart").op(Op::ChartUpsert {
        id: None,
        chart: rec.clone(),
    }))
    .unwrap();
    s.apply(TxnInput::new("ins").op(Op::Structure(StructOp::InsertRows {
        sheet: S1,
        at: 0,
        n: 2,
    })))
    .unwrap();
    let wb = s.load().unwrap();
    let moved = wb.charts.values().next().unwrap();
    assert_eq!(moved.row, 5);
    assert_eq!(
        ChartSpec::parse(&moved.spec).unwrap().series[0].range.rect,
        Rect::new(4, 1, 11, 1)
    );
    s.apply(TxnInput::new("del").op(Op::Structure(StructOp::DeleteRows {
        sheet: S1,
        at: 5,
        n: 1,
    })))
    .unwrap();
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.load().unwrap().charts.values().next().unwrap(), &rec);
}
