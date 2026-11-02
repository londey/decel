// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! A 10 k-cell `SetCells` and its undo must each take under 100 ms (Phase 1).
//!
//! Every iteration rewrites the same 10 k cells, so history grows by 10 k
//! rows per iteration; the WAL is checkpointed between iterations, as the
//! engine does when idle.

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use decel_model::{Cell, CellAddr, Op, SheetId, TxnInput, Value};
use decel_store::Store;

fn fill(n: u32, base: i64) -> TxnInput {
    let cells = (0..n)
        .map(|i| {
            (
                CellAddr::new(SheetId(1), i / 20, i % 20),
                Cell::value(Value::Int(base + i64::from(i))),
            )
        })
        .collect();
    TxnInput::new("fill").op(Op::SetCells(cells))
}

fn bench(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(&dir.path().join("bench.decel")).unwrap();
    let mut base = 0;
    c.bench_function("set_cells_10k", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                store.checkpoint().unwrap();
                base += 1;
                let input = fill(10_000, base);
                let t = Instant::now();
                store.apply(input).unwrap();
                total += t.elapsed();
            }
            total
        })
    });
    c.bench_function("undo_10k", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                store.checkpoint().unwrap();
                let t = Instant::now();
                store.undo().unwrap();
                total += t.elapsed();
                store.redo().unwrap();
            }
            total
        })
    });
}

/// Per-cell history must take under 10 ms with 1 M `cell_changes` rows.
fn history(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::create(&dir.path().join("history.decel")).unwrap();
    for base in 0..100 {
        store.apply(fill(10_000, base)).unwrap();
        store.checkpoint().unwrap();
    }
    let target = CellAddr::new(SheetId(1), 250, 7);
    assert_eq!(store.cell_history(target).unwrap().len(), 100);
    c.bench_function("cell_history_1m_rows", |b| {
        b.iter(|| store.cell_history(target).unwrap())
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench, history
}
criterion_main!(benches);
