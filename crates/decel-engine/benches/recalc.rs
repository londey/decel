// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Phase 2 targets: one edit with 1 k dependents under 16 ms; full recalc of
//! 100 k computed cells under 1 s.

use criterion::{Criterion, criterion_group, criterion_main};
use decel_engine::testkit::{Wb, meta};
use decel_model::{Cell, Value};

fn bench(c: &mut Criterion) {
    let mut wb = Wb::new();
    wb.set(1, 1, 1);
    wb.fill_meta(
        1,
        1000,
        2,
        &meta().with("value", "return c:at(1, 1).value + c.row"),
    );
    wb.recalc();
    let a1 = wb.addr(1, 1);
    let mut n = 0i64;
    c.bench_function("edit_with_1k_dependents", |b| {
        b.iter(|| {
            n += 1;
            wb.apply_cells(vec![(a1, Cell::value(Value::Int(n)))]);
            wb.engine.recalc_all();
        })
    });

    let mut big = Wb::new();
    let cells = (0..100_000u32)
        .map(|r| (big.addr(r + 1, 1), Cell::value(Value::Int(i64::from(r)))))
        .collect();
    big.apply_cells(cells);
    big.fill_meta(
        1,
        100_000,
        2,
        &meta().with("value", "return c:left().value * 2"),
    );
    c.bench_function("full_recalc_100k", |b| {
        b.iter(|| {
            big.engine.mark_all_dirty();
            big.engine.recalc_all();
        })
    });
}

/// §11: a 100 k-cell workbook opened and displayed in under 1 s.
fn open(c: &mut Criterion) {
    use decel_model::{CellAddr, MetaSource, Op, SheetId, TxnInput};
    use decel_store::Store;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("open.decel");
    {
        let mut store = Store::create(&path).unwrap();
        let m = MetaSource::default().with("value", "return c:left().value * 2");
        let h = m.hash();
        let mut cells = Vec::new();
        for r in 0..50_000u32 {
            cells.push((
                CellAddr::new(SheetId(1), r, 0),
                Cell::value(Value::Int(i64::from(r))),
            ));
            cells.push((CellAddr::new(SheetId(1), r, 1), Cell::EMPTY.with_meta(h)));
        }
        store
            .apply(TxnInput::new("big").op(Op::SetCells(cells)).meta(m))
            .unwrap();
        store.checkpoint().unwrap();
    }
    c.bench_function("open_and_display_100k", |b| {
        b.iter(|| {
            let store = Store::open_read_only(&path).unwrap();
            let wb = store.load().unwrap();
            let mut engine = decel_engine::Engine::new(wb, Default::default()).unwrap();
            // A screenful: 50 rows x 20 columns.
            for r in 0..50 {
                for col in 0..20 {
                    let _ = engine.display(CellAddr::new(SheetId(1), r, col));
                }
            }
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench, open
}
criterion_main!(benches);
