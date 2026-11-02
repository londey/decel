// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Property tests for the store (Phase 1 acceptance criteria).

use std::collections::BTreeMap;

use decel_model::{
    Axis, Cell, CellAddr, ChartId, ChartRecord, MetaSource, Op, Sheet, SheetId, StructOp, TxnInput,
    Value, Workbook,
};
use decel_store::Store;
use proptest::prelude::*;

/// (sheet, row, col, value, meta)
type ACell = (u8, u8, u8, Option<i8>, Option<u8>);

#[derive(Debug, Clone)]
enum AOp {
    Set(Vec<ACell>),
    AddSheet(u8, Option<u8>),
    Rename(u8, u8),
    Move(u8, u8),
    Delete(u8),
    Layout(u8, bool, u8, Option<u8>),
    ChartUpsert(u8, Option<u8>, u8, u8),
    ChartDelete(u8),
    Structure(u8, bool, bool, u8, u8),
    Undo,
    Redo,
}

fn aop() -> impl Strategy<Value = AOp> {
    let cell = (
        any::<u8>(),
        0u8..12,
        0u8..6,
        any::<Option<i8>>(),
        proptest::option::of(0u8..3),
    );
    prop_oneof![
        8 => proptest::collection::vec(cell, 1..8).prop_map(AOp::Set),
        1 => (0u8..5, any::<Option<u8>>()).prop_map(|(n, p)| AOp::AddSheet(n, p)),
        1 => (any::<u8>(), 0u8..5).prop_map(|(s, n)| AOp::Rename(s, n)),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(s, p)| AOp::Move(s, p)),
        1 => any::<u8>().prop_map(AOp::Delete),
        1 => (any::<u8>(), any::<bool>(), 0u8..8, any::<Option<u8>>())
            .prop_map(|(s, r, i, z)| AOp::Layout(s, r, i, z)),
        1 => (any::<u8>(), any::<Option<u8>>(), 0u8..10, 0u8..5)
            .prop_map(|(s, e, r, c)| AOp::ChartUpsert(s, e, r, c)),
        1 => any::<u8>().prop_map(AOp::ChartDelete),
        2 => (any::<u8>(), any::<bool>(), any::<bool>(), 0u8..12, 1u8..4)
            .prop_map(|(s, rows, del, at, n)| AOp::Structure(s, rows, del, at, n)),
        2 => Just(AOp::Undo),
        1 => Just(AOp::Redo),
    ]
}

const NAMES: [&str; 5] = ["A", "B", "C", "D", "Sheet1"];

fn metas() -> Vec<MetaSource> {
    (0..3)
        .map(|i| {
            MetaSource::default()
                .with("kind", &format!("\"k{i}\""))
                .with("value", &format!("return {i}"))
        })
        .collect()
}

fn pick_sheet(wb: &Workbook, i: u8) -> SheetId {
    let ids: Vec<SheetId> = wb.sheets.keys().copied().collect();
    ids[usize::from(i) % ids.len()]
}

/// Resolves an abstract op against the current state.
fn concrete(wb: &Workbook, op: &AOp) -> Option<TxnInput> {
    let metas = metas();
    let op = match op {
        AOp::Set(cells) => Op::SetCells(
            cells
                .iter()
                .map(|(s, r, c, v, m)| {
                    (
                        CellAddr::new(pick_sheet(wb, *s), u32::from(*r), u32::from(*c)),
                        Cell {
                            raw: v.map(|v| Value::Int(i64::from(v))),
                            meta: m.map(|m| metas[usize::from(m)].hash()),
                        },
                    )
                })
                .collect(),
        ),
        AOp::AddSheet(n, p) => Op::AddSheet {
            name: NAMES[usize::from(*n)].into(),
            position: p.map(|p| u32::from(p % 4)),
        },
        AOp::Rename(s, n) => Op::RenameSheet {
            id: pick_sheet(wb, *s),
            name: NAMES[usize::from(*n)].into(),
        },
        AOp::Move(s, p) => Op::MoveSheet {
            id: pick_sheet(wb, *s),
            position: u32::from(*p % 5),
        },
        AOp::Delete(s) => Op::DeleteSheet {
            id: pick_sheet(wb, *s),
        },
        AOp::Layout(s, rows, i, z) => Op::SetLayout {
            sheet: pick_sheet(wb, *s),
            axis: if *rows { Axis::Row } else { Axis::Col },
            idx: u32::from(*i),
            size: z.map(|z| f64::from(z) + 1.0),
        },
        AOp::ChartUpsert(s, existing, r, c) => {
            let ids: Vec<ChartId> = wb.charts.keys().copied().collect();
            let id = existing.and_then(|e| ids.get(usize::from(e) % ids.len().max(1)).copied());
            Op::ChartUpsert {
                id,
                chart: ChartRecord {
                    sheet: pick_sheet(wb, *s),
                    row: u32::from(*r),
                    col: u32::from(*c),
                    spec: format!("{{\"n\":{r}}}"),
                },
            }
        }
        AOp::ChartDelete(i) => {
            let ids: Vec<ChartId> = wb.charts.keys().copied().collect();
            let id = *ids.get(usize::from(*i) % ids.len().max(1))?;
            Op::ChartDelete { id }
        }
        AOp::Structure(s, rows, del, at, n) => {
            let (sheet, at, n) = (pick_sheet(wb, *s), u32::from(*at), u32::from(*n));
            Op::Structure(match (rows, del) {
                (true, false) => StructOp::InsertRows { sheet, at, n },
                (true, true) => StructOp::DeleteRows { sheet, at, n },
                (false, false) => StructOp::InsertCols { sheet, at, n },
                (false, true) => StructOp::DeleteCols { sheet, at, n },
            })
        }
        AOp::Undo | AOp::Redo => return None,
    };
    Some(TxnInput {
        label: "op".into(),
        ops: vec![op],
        metas,
    })
}

/// The comparable state: sheets and charts (metatables are never deleted).
fn state(wb: &Workbook) -> (BTreeMap<SheetId, Sheet>, BTreeMap<ChartId, ChartRecord>) {
    (wb.sheets.clone(), wb.charts.clone())
}

/// Runs one abstract op, keeping `mirror` up to date from the returned
/// changes. Returns whether anything was committed.
fn step(store: &mut Store, mirror: &mut Workbook, op: &AOp) -> bool {
    let committed = match op {
        AOp::Undo => store.undo().unwrap(),
        AOp::Redo => store.redo().unwrap(),
        other => {
            let current = store.load().unwrap();
            match concrete(&current, other).map(|t| store.apply(t)) {
                Some(Ok(c)) => c,
                // Invalid ops (duplicate names, deleting the last sheet…)
                // are rejected without effect.
                Some(Err(_)) | None => None,
            }
        }
    };
    match committed {
        Some(c) => {
            mirror.apply_changes(&c.changes);
            true
        }
        None => false,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn undo_all_then_redo_all(ops in proptest::collection::vec(aop(), 1..200)) {
        let mut store = Store::in_memory().unwrap();
        let mut mirror = store.load().unwrap();
        let initial = state(&mirror);
        for op in &ops {
            step(&mut store, &mut mirror, op);
            // The returned changes reproduce the stored state exactly.
            prop_assert_eq!(state(&mirror), state(&store.load().unwrap()));
        }
        let fin = state(&store.load().unwrap());
        let sets_values = ops
            .iter()
            .any(|o| matches!(o, AOp::Set(c) if c.iter().any(|x| x.3.is_some())));
        if sets_values {
            prop_assert!(!store.txns().unwrap().is_empty());
        }
        let mut undone = 0;
        while store.undo().unwrap().is_some() {
            undone += 1;
        }
        prop_assert_eq!(&state(&store.load().unwrap()), &initial);
        for _ in 0..undone {
            prop_assert!(store.redo().unwrap().is_some());
        }
        prop_assert_eq!(state(&store.load().unwrap()), fin);
    }

    #[test]
    fn reopen_preserves_state_and_stacks(
        ops in proptest::collection::vec(aop(), 1..120),
        at in any::<proptest::sample::Index>(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.decel");
        let mut store = Store::create(&path).unwrap();
        let mut mirror = store.load().unwrap();
        let reopen_at = at.index(ops.len());
        for (i, op) in ops.iter().enumerate() {
            step(&mut store, &mut mirror, op);
            if i == reopen_at {
                let wb = state(&store.load().unwrap());
                let (undo, redo) = (store.undo_stack().to_vec(), store.redo_stack().to_vec());
                drop(store);
                store = Store::open(&path).unwrap();
                prop_assert_eq!(state(&store.load().unwrap()), wb);
                prop_assert_eq!(store.undo_stack(), undo.as_slice());
                prop_assert_eq!(store.redo_stack(), redo.as_slice());
            }
        }
    }

    #[test]
    fn replaying_history_reproduces_state(ops in proptest::collection::vec(aop(), 1..150)) {
        let mut store = Store::in_memory().unwrap();
        let mut replay = store.load().unwrap();
        let mut mirror = replay.clone();
        for op in &ops {
            step(&mut store, &mut mirror, op);
        }
        for txn in store.txns().unwrap() {
            replay.apply_changes(&store.txn_changes(txn.id).unwrap());
        }
        prop_assert_eq!(state(&replay), state(&store.load().unwrap()));
    }
}
