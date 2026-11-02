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

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench
}
criterion_main!(benches);
