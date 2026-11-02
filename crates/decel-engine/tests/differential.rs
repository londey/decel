// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Differential fuzz test: incremental recalculation after random edits must
//! give the same final values as a full recalculation from scratch.

use decel_engine::testkit::{Wb, meta};
use decel_engine::{Engine, Limits};
use decel_model::{CellAddr, MetaSource, Value};
use proptest::prelude::*;

const ROWS: u32 = 8;
const COLS: u32 = 4;

/// Formulas mixing relative reads, ranges, conditional reads (so
/// dependencies change between evaluations), possible cycles and errors.
fn metas() -> Vec<MetaSource> {
    [
        "return (c:up().value or 0) + 1",
        "return c:left().value",
        "return c:above():sum()",
        "local h = c:at(1, 1)\nif h.ok and type(h.value) == 'number' and h.value > 0 then return c:right().value end\nreturn 0",
        "return c:column():count()",
        "return c:down().value",
        "error('boom')",
        "local s = 0\nfor h in c:left_of():cells() do\n  if h.ok and type(h.value) == 'number' then s = s + h.value end\nend\nreturn s",
        "return c.sheet:last_row()",
        "if c:right().ok then return 1 end return 2",
        "return range.sum(c:rect(-1, -1, 1, 1))",
    ]
    .iter()
    .enumerate()
    .map(|(i, v)| {
        meta()
            .with("kind", &format!("\"k{}\"", i % 3))
            .with("value", v)
    })
    .chain(std::iter::once(
        meta().with("validate", "if type(v) == 'number' and v > 5 then return false, 'big' end return true"),
    ))
    .collect()
}

#[derive(Debug, Clone)]
enum Edit {
    Set(u32, u32, i8),
    Clear(u32, u32),
    Meta(u32, u32, usize),
    NoMeta(u32, u32),
}

fn edit() -> impl Strategy<Value = Edit> {
    let r = 1..=ROWS;
    let c = 1..=COLS;
    prop_oneof![
        3 => (r.clone(), c.clone(), any::<i8>()).prop_map(|(r, c, v)| Edit::Set(r, c, v)),
        1 => (r.clone(), c.clone()).prop_map(|(r, c)| Edit::Clear(r, c)),
        3 => (r.clone(), c.clone(), 0..12usize).prop_map(|(r, c, m)| Edit::Meta(r, c, m)),
        1 => (r, c).prop_map(|(r, c)| Edit::NoMeta(r, c)),
    ]
}

/// A comparable final value: errors compare by kind only, because which
/// cell of an interlocking cycle reports it first depends on order.
fn key(v: Value) -> String {
    match v {
        Value::Error(e) => e.kind.code().to_string(),
        Value::Num(n) => format!("num:{}", n.to_bits()),
        other => format!("{other:?}"),
    }
}

fn snapshot(engine: &mut Engine, sheet: decel_model::SheetId) -> Vec<String> {
    let mut out = Vec::new();
    for r in 0..ROWS {
        for c in 0..COLS {
            let a = CellAddr::new(sheet, r, c);
            out.push(key(engine.final_value(a)));
            out.push(engine.warning(a).map(|w| w.to_string()).unwrap_or_default());
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn incremental_equals_full(
        edits in proptest::collection::vec(edit(), 1..40),
        recalc_mask in proptest::collection::vec(any::<bool>(), 40),
    ) {
        let metas = metas();
        let mut wb = Wb::new();
        for (i, e) in edits.iter().enumerate() {
            match e {
                Edit::Set(r, c, v) => { wb.set(*r, *c, i64::from(*v)); }
                Edit::Clear(r, c) => { wb.clear(*r, *c); }
                Edit::Meta(r, c, m) => { wb.meta(*r, *c, &metas[*m]); }
                Edit::NoMeta(r, c) => { wb.remove_meta(*r, *c); }
            }
            // Sometimes recalc between edits, sometimes let dirt pile up.
            if recalc_mask[i] {
                wb.recalc();
            }
        }
        wb.recalc();
        let incremental = snapshot(&mut wb.engine, wb.sheet);

        let mut fresh = Engine::new(wb.engine.workbook().clone(), Limits::default()).unwrap();
        fresh.recalc_all();
        let full = snapshot(&mut fresh, wb.sheet);
        prop_assert_eq!(incremental, full);
    }
}
