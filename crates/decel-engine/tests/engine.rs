// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Phase 2 acceptance tests, driven through the headless test kit.

use std::time::Duration;

use decel_engine::testkit::{Wb, meta};
use decel_engine::{Align, Limits, MemberStatus};
use decel_model::{ErrorKind, MetaSource, Value};

fn latitude() -> MetaSource {
    meta()
        .with("kind", "\"latitude\"")
        .with("unit", "\"deg\"")
        .with("display", "return fmt.dms(v, \"lat\", 1)")
        .with("edit", "return string.format(\"%.6f\", v)")
        .with(
            "parse",
            "local n = tonumber(text) or parse.dms(text)\n\
             if not n then return nil, \"Enter decimal degrees or D°M'S\\\" N/S\" end\n\
             return n",
        )
        .with(
            "validate",
            "if type(v) ~= \"number\" or v < -90 or v > 90 then return false, \"Latitude must be within ±90°\" end\n\
             return true",
        )
        .with("tooltip", "return string.format(\"%.6f° — %s\", v, fmt.dms(v, \"lat\", 2))")
        .with("edit_hint", "return \"Decimal degrees (−35.3075) or 35°18'27\\\"S\"")
}

fn running_total() -> MetaSource {
    meta()
        .with("kind", "\"running total\"")
        .with(
            "value",
            "local prev = c:up()\n\
             local base = (prev.row >= 1 and prev.kind == self.kind) and prev.value or 0\n\
             return base + c:left().value",
        )
        .with("style", "{ align = \"right\", mono = true }")
}

fn err_kind(v: &Value) -> Option<ErrorKind> {
    match v {
        Value::Error(e) => Some(e.kind),
        _ => None,
    }
}

#[test]
fn latitude_example() {
    let mut wb = Wb::new();
    wb.meta(2, 2, &latitude());
    wb.type_text(2, 2, "-35.3075").unwrap();
    assert_eq!(wb.value(2, 2), Value::Num(-35.3075));
    assert_eq!(wb.display(2, 2), "35°18′27.0″S");
    assert_eq!(wb.edit_text(2, 2), "-35.307500");
    assert_eq!(wb.tooltip(2, 2).unwrap(), "-35.307500° — 35°18′27.00″S");
    assert!(wb.edit_hint(2, 2).unwrap().starts_with("Decimal degrees"));

    // DMS text is accepted by parse.
    wb.type_text(2, 2, "35°18'27\"S").unwrap();
    assert!((wb.value(2, 2).as_f64().unwrap() + 35.3075).abs() < 1e-9);

    // 95 is rejected by validate; garbage by parse. Nothing changes.
    let e = wb.type_text(2, 2, "95").unwrap_err();
    assert_eq!(e.message, "Latitude must be within ±90°");
    let e = wb.type_text(2, 2, "north-ish").unwrap_err();
    assert!(
        e.message.starts_with("Enter decimal degrees"),
        "{}",
        e.message
    );
    assert!((wb.value(2, 2).as_f64().unwrap() + 35.3075).abs() < 1e-9);

    let d = wb.display_cell(2, 2);
    assert_eq!(d.kind.as_deref(), Some("latitude"));
    assert_eq!(d.unit.as_deref(), Some("deg"));
}

#[test]
fn running_total_filled_500_rows() {
    let mut wb = Wb::new();
    for r in 1..=500 {
        wb.set(r, 1, i64::from(r));
    }
    wb.fill_meta(1, 500, 2, &running_total());
    assert_eq!(wb.value(1, 2), Value::Int(1));
    assert_eq!(wb.value(500, 2), Value::Int(500 * 501 / 2));
    let d = wb.display_cell(500, 2);
    assert_eq!(d.style.align, Some(Align::Right));
    assert_eq!(d.style.mono, Some(true));
    assert_eq!(d.hash, d.hash.and(wb.display_cell(1, 2).hash));

    // An edit at the top flows to the bottom.
    wb.set(1, 1, 101);
    assert_eq!(wb.value(500, 2), Value::Int(500 * 501 / 2 + 100));
}

#[test]
fn running_total_evaluates_bottom_up_without_stack_overflow() {
    // Demand the last cell first: the chain is resolved through the
    // engine's explicit stack, not Lua recursion.
    let mut wb = Wb::new();
    for r in 1..=2000 {
        wb.set(r, 1, 1);
    }
    wb.fill_meta(1, 2000, 2, &running_total());
    let a = wb.addr(2000, 2);
    assert_eq!(wb.engine.final_value(a), Value::Int(2000));
}

#[test]
fn cycles_mark_members_only_and_recover() {
    let mut wb = Wb::new();
    let next = meta().with("value", "return c:right().value + 1");
    let back = meta().with("value", "return c:offset(0, -2).value + 1");
    wb.meta(1, 1, &next).meta(1, 2, &next).meta(1, 3, &back); // A→B→C→A
    wb.set(5, 1, 7);
    let outside = meta().with("value", "return c:up().value * 2");
    wb.meta(6, 1, &outside); // independent of the cycle
    let reader = meta().with("value", "return c:at(1, 1).value");
    wb.meta(8, 1, &reader); // depends on the cycle

    for col in 1..=3 {
        assert_eq!(
            err_kind(&wb.value(1, col)),
            Some(ErrorKind::Cycle),
            "col {col}"
        );
    }
    assert_eq!(wb.value(6, 1), Value::Int(14));
    // A dependent sees the error propagated, but isn't itself in the cycle.
    assert!(wb.value(8, 1).is_error());

    // Break the cycle: C now reads a constant.
    wb.meta(1, 3, &meta().with("value", "return 10"));
    assert_eq!(wb.value(1, 3), Value::Int(10));
    assert_eq!(wb.value(1, 2), Value::Int(11));
    assert_eq!(wb.value(1, 1), Value::Int(12));
    assert_eq!(wb.value(8, 1), Value::Int(12));
}

#[test]
fn self_reference_is_a_cycle() {
    let mut wb = Wb::new();
    wb.meta(1, 1, &meta().with("value", "return c:offset(0, 0).value"));
    assert_eq!(err_kind(&wb.value(1, 1)), Some(ErrorKind::Cycle));
}

#[test]
fn timeout_and_memory_errors_are_contained() {
    let mut wb = Wb::with_limits(Limits {
        instructions: 2_000_000,
        time: Duration::from_millis(200),
        memory: 64 * 1024 * 1024,
    });
    wb.set(1, 1, 3);
    wb.meta(1, 2, &meta().with("value", "while true do end"));
    wb.meta(
        1,
        3,
        &meta().with("value", "return string.rep(\"x\", 1e10)"),
    );
    wb.meta(
        1,
        4,
        &meta().with("value", "local t = {} for i = 1, 1e9 do t[i] = i end"),
    );
    wb.meta(
        1,
        5,
        &meta().with("value", "return c:offset(0, -4).value + 1"),
    );
    assert_eq!(err_kind(&wb.value(1, 2)), Some(ErrorKind::Timeout));
    assert_eq!(err_kind(&wb.value(1, 3)), Some(ErrorKind::Memory));
    assert!(matches!(
        err_kind(&wb.value(1, 4)),
        Some(ErrorKind::Memory | ErrorKind::Timeout)
    ));
    assert_eq!(wb.value(1, 5), Value::Int(4));
}

#[test]
fn sandbox_escapes_fail_or_are_harmless() {
    let mut wb = Wb::new();
    let attempts = [
        "return io.open('/etc/passwd')",
        "return os.getenv('HOME')",
        "return load('return 1')()",
        "return require('io')",
        "return debug.traceback()",
        "return string.dump(function() end)",
        "getmetatable('').__index = { upper = function() return 'pwned' end } return 1",
        "local mt = getmetatable(c) mt.__index = nil return 1",
        "x_global = 1 return 1",
        "self.kind = 'hacked' return 1",
    ];
    for (i, code) in attempts.iter().enumerate() {
        let row = i as u32 + 1;
        wb.meta(row, 1, &meta().with("kind", "\"k\"").with("value", code));
        assert!(wb.value(row, 1).is_error(), "{code} should fail");
    }
    // Nothing leaked into shared state.
    wb.meta(
        20,
        1,
        &meta().with("value", "return ('a'):upper() .. tostring(x_global)"),
    );
    assert_eq!(wb.value(20, 1), Value::str("Anil"));
    assert_eq!(wb.display_cell(1, 1).kind.as_deref(), Some("k"));
}

#[test]
fn range_dependency_sees_newly_populated_cells() {
    let mut wb = Wb::new();
    wb.set(1, 1, 1).set(3, 1, 3);
    wb.meta(10, 1, &meta().with("value", "return c:above():sum()"));
    assert_eq!(wb.value(10, 1), Value::Int(4));
    wb.set(2, 1, 2); // a blank above is filled
    assert_eq!(wb.value(10, 1), Value::Int(6));
    wb.clear(1, 1);
    assert_eq!(wb.value(10, 1), Value::Int(5));
    // Column totals exclude self.
    wb.meta(11, 1, &meta().with("value", "return c:column():count()"));
    assert_eq!(wb.value(11, 1), Value::Int(3));
}

#[test]
fn errors_carry_member_and_line_and_propagate() {
    let mut wb = Wb::new();
    wb.meta(1, 1, &meta().with("value", "local x = 1\nreturn x + nil"));
    match wb.value(1, 1) {
        Value::Error(e) => {
            assert_eq!(e.kind, ErrorKind::Lua);
            assert_eq!(e.member.as_deref(), Some("value"));
            assert_eq!(e.line, Some(2));
        }
        other => panic!("{other:?}"),
    }
    // Reading an error re-raises it; h.ok lets code handle it.
    wb.meta(2, 1, &meta().with("value", "return c:up().value"));
    assert!(wb.value(2, 1).is_error());
    wb.meta(
        3,
        1,
        &meta().with(
            "value",
            "local h = c:at(1, 1) if h.ok then return 1 end return h.err ~= nil",
        ),
    );
    assert_eq!(wb.value(3, 1), Value::Bool(true));
    wb.meta(
        4,
        1,
        &meta().with(
            "value",
            "local ok = pcall(function() return c:at(1,1).value end) return ok",
        ),
    );
    assert_eq!(wb.value(4, 1), Value::Bool(false));
}

#[test]
fn syntax_errors_are_reported_per_member() {
    let mut wb = Wb::new();
    let src = meta()
        .with("value", "return 1 +")
        .with("display", "return 'ok'\nend end");
    wb.meta(1, 1, &src);
    let a = wb.addr(1, 1);
    let errors = wb.engine.compile_errors(&src.hash());
    assert_eq!(errors.len(), 2);
    assert!(
        errors
            .iter()
            .all(|(_, e)| e.line == Some(1) || e.line == Some(2))
    );
    assert!(wb.value(1, 1).is_error());
    let _ = a;
}

#[test]
fn display_errors_fall_back_to_default() {
    let mut wb = Wb::new();
    wb.set(1, 1, 42);
    wb.meta(1, 1, &meta().with("display", "error('nope')"));
    let d = wb.display_cell(1, 1);
    assert_eq!(d.text, "42");
    assert!(d.display_error.is_some());
    assert!(d.has_marker());
}

#[test]
fn display_tables_and_style_precedence() {
    let mut wb = Wb::new();
    wb.set(1, 1, -5);
    wb.meta(
        1,
        1,
        &meta()
            .with("style", "{ align = 'centre', bold = true, fg = '#000000' }")
            .with("display", "if v < 0 then return { text = '(' .. -v .. ')', fg = '#c0392b' } end return tostring(v)"),
    );
    let d = wb.display_cell(1, 1);
    assert_eq!(d.text, "(5)");
    assert_eq!(d.style.fg, Some((0xc0, 0x39, 0x2b)));
    assert_eq!(d.style.align, Some(Align::Centre));
    assert_eq!(d.style.bold, Some(true));
    // Defaults: numbers right, text left.
    wb.set(2, 1, 3).set(3, 1, "x");
    assert_eq!(wb.display_cell(2, 1).style.align, Some(Align::Right));
    assert_eq!(wb.display_cell(3, 1).style.align, Some(Align::Left));
}

#[test]
fn computed_validation_gives_warning_not_rejection() {
    let mut wb = Wb::new();
    wb.set(1, 1, 5);
    wb.meta(
        1,
        2,
        &meta().with("value", "return c:left().value * 10").with(
            "validate",
            "if v > 100 then return false, 'too big' end return true",
        ),
    );
    assert_eq!(wb.value(1, 2), Value::Int(50));
    assert!(wb.display_cell(1, 2).warning.is_none());
    wb.set(1, 1, 50);
    assert_eq!(wb.value(1, 2), Value::Int(500));
    assert_eq!(wb.display_cell(1, 2).warning.as_deref(), Some("too big"));
    assert!(wb.tooltip(1, 2).unwrap().contains("too big"));
}

#[test]
fn sheet_named_rename_gives_ref_error_and_recovers() {
    let mut wb = Wb::new();
    let inputs = wb.add_sheet("Inputs");
    wb.engine.apply(&[decel_model::Change::Cell {
        addr: decel_model::CellAddr::new(inputs, 0, 0),
        before: Default::default(),
        after: decel_model::Cell::value(Value::Int(9)),
    }]);
    wb.meta(
        1,
        1,
        &meta().with("value", "return c:sheet_named('Inputs'):at(1, 1).value"),
    );
    assert_eq!(wb.value(1, 1), Value::Int(9));
    wb.rename_sheet(inputs, "Params");
    assert_eq!(err_kind(&wb.value(1, 1)), Some(ErrorKind::Ref));
    wb.rename_sheet(inputs, "Inputs");
    assert_eq!(wb.value(1, 1), Value::Int(9));
}

#[test]
fn helpers_fields_and_frozen_self() {
    let mut wb = Wb::new();
    wb.set(1, 1, 4);
    wb.meta(
        1,
        2,
        &meta()
            .with("factor", "3")
            .with("triple", "function(self, x) return x * self.factor end")
            .with("value", "return self:triple(c:left().value)"),
    );
    assert_eq!(wb.value(1, 2), Value::Int(12));
}

#[test]
fn print_goes_to_cell_log() {
    let mut wb = Wb::new();
    wb.meta(1, 1, &meta().with("value", "print('hello', 1) return 0"));
    wb.recalc();
    let a = wb.addr(1, 1);
    assert_eq!(wb.engine.log(a), ["hello\t1"]);
}

#[test]
fn preview_never_changes_the_workbook() {
    let mut wb = Wb::new();
    wb.set(1, 1, 2);
    wb.meta(1, 2, &meta().with("value", "return c:left().value"));
    let a = wb.addr(1, 2);
    let edited = meta()
        .with("kind", "\"double\"")
        .with("value", "print('p') return c:left().value * 2")
        .with("display", "return 'x' .. v");
    let p = wb.engine.preview(a, &edited).unwrap();
    assert_eq!(p.value, Value::Int(4));
    assert_eq!(p.display.text, "x4");
    assert_eq!(p.display.kind.as_deref(), Some("double"));
    assert_eq!(p.log, ["p"]);
    assert!(p.members.iter().all(|(_, s)| *s == MemberStatus::Ok));
    assert_eq!(wb.value(1, 2), Value::Int(2));

    let broken = meta()
        .with("value", "return c:left().value +")
        .with("display", "error('d')");
    let p = wb.engine.preview(a, &broken).unwrap();
    assert!(matches!(p.members[0].1, MemberStatus::Syntax(_)));
    let with_raw = wb.addr(1, 1);
    let p = wb
        .engine
        .preview(with_raw, &meta().with("display", "error('d')"))
        .unwrap();
    assert!(matches!(p.members[0].1, MemberStatus::Runtime(_)));
}

#[test]
fn range_api() {
    let mut wb = Wb::new();
    for (r, v) in [(1, 1), (2, 2), (3, 3), (4, 4)] {
        wb.set(r, 1, v);
    }
    wb.set(5, 1, "text");
    let cases = [
        ("return c:above():count()", Value::Int(5)),
        ("return c:above():sum()", Value::Int(10)),
        ("return c:above():avg()", Value::Num(2.5)),
        ("return c:above():max()", Value::Num(4.0)),
        ("return #c:above():values()", Value::Int(5)),
        (
            "local n = 0 for h in c:above():cells() do n = n + 1 end return n",
            Value::Int(5),
        ),
        (
            "return c:above():reduce(function(a, v) return a + (tonumber(v) or 0) end, 0)",
            Value::Int(10),
        ),
        (
            "return #c:above():filter(function(h) return type(h.value) == 'number' end)",
            Value::Int(4),
        ),
        (
            "return c:above():map(function(h) return h.row end)[5]",
            Value::Int(5),
        ),
        ("return c:above().rows", Value::Int(9)),
        ("return range.sum(c:above(), 100)", Value::Int(110)),
        ("return c.sheet:range(1, 1, 2, 1):sum()", Value::Int(3)),
        ("return c.sheet:last_row()", Value::Int(10)),
        ("return c:rect(-9, 0, -8, 0):sum()", Value::Int(3)),
        ("return c.row * 100 + c.col", Value::Int(1001)),
    ];
    for (code, want) in cases {
        wb.meta(10, 1, &meta().with("value", code));
        assert_eq!(wb.value(10, 1), want, "{code}");
    }
}

#[test]
fn where_kind_filters() {
    let mut wb = Wb::new();
    let lat = meta().with("kind", "\"lat\"");
    let lon = meta().with("kind", "\"lon\"");
    wb.set(1, 1, 10).set(2, 1, 20).set(3, 1, 30);
    wb.meta(1, 1, &lat).meta(2, 1, &lon).meta(3, 1, &lat);
    wb.meta(
        4,
        1,
        &meta().with("value", "return c:above():where_kind('lat'):sum()"),
    );
    assert_eq!(wb.value(4, 1), Value::Int(40));
}

fn fresh_values(wb: &mut Wb, rows: u32, col: u32) -> Vec<Value> {
    let mut fresh =
        decel_engine::Engine::new(wb.engine.workbook().clone(), Limits::default()).unwrap();
    fresh.recalc_all();
    (1..=rows)
        .map(|r| fresh.final_value(wb.addr(r, col)))
        .collect()
}

#[test]
fn inserting_rows_inside_running_total_keeps_totals_correct() {
    let mut wb = Wb::new();
    for r in 1..=500 {
        wb.set(r, 1, 1);
    }
    wb.fill_meta(1, 500, 2, &running_total());
    assert_eq!(wb.value(500, 2), Value::Int(500));
    // Insert 3 blank rows before row 201 (0-based 200).
    wb.structure(decel_model::StructOp::InsertRows {
        sheet: wb.sheet,
        at: 200,
        n: 3,
    });
    wb.recalc();
    // Above the gap: unchanged. The gap is empty. Below it, c:up() sees a
    // blank row (no kind), so the total restarts from that row's amount.
    assert_eq!(wb.value(200, 2), Value::Int(200));
    for r in 201..=203 {
        assert_eq!(wb.value(r, 2), Value::Nil);
    }
    assert_eq!(wb.value(204, 2), Value::Int(1));
    assert_eq!(wb.value(503, 2), Value::Int(300));
    // Incremental equals a fresh full recalculation.
    let incremental: Vec<Value> = (1..=503).map(|r| wb.value(r, 2)).collect();
    assert_eq!(incremental, fresh_values(&mut wb, 503, 2));
}

#[test]
fn deleting_a_row_in_a_range_dependency_recalculates() {
    let mut wb = Wb::new();
    for r in 1..=10 {
        wb.set(r, 1, i64::from(r));
    }
    wb.meta(12, 1, &meta().with("value", "return c:above():sum()"));
    wb.meta(12, 2, &meta().with("value", "return c:offset(0, -1).value"));
    assert_eq!(wb.value(12, 1), Value::Int(55));
    wb.structure(decel_model::StructOp::DeleteRows {
        sheet: wb.sheet,
        at: 4, // row 5 (value 5)
        n: 1,
    });
    // The total moved up to row 11 and lost the 5.
    assert_eq!(wb.value(11, 1), Value::Int(50));
    assert_eq!(wb.value(11, 2), Value::Int(50));
    assert_eq!(wb.value(12, 1), Value::Nil);
    // Delete a column left of a relative reference: it still points left.
    wb.set(11, 4, 7);
    wb.meta(11, 5, &meta().with("value", "return c:left().value"));
    wb.structure(decel_model::StructOp::DeleteCols {
        sheet: wb.sheet,
        at: 2, // column 3 (empty)
        n: 1,
    });
    assert_eq!(wb.value(11, 4), Value::Int(7));
}

#[test]
fn absolute_at_is_not_rewritten_by_structure_ops() {
    let mut wb = Wb::new();
    wb.set(1, 1, 5).set(2, 1, 9);
    wb.meta(5, 2, &meta().with("value", "return c:at(1, 1).value"));
    assert_eq!(wb.value(5, 2), Value::Int(5));
    wb.structure(decel_model::StructOp::InsertRows {
        sheet: wb.sheet,
        at: 0,
        n: 1,
    });
    // c:at(1,1) still means row 1, which is now blank (DESIGN §7, documented).
    assert_eq!(wb.value(6, 2), Value::Nil);
}
