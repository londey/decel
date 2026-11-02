// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Robustness: schema-valid but hostile workbook files must never panic the
//! store or the engine (DESIGN §11). Errors are fine; panics are not.

use decel_engine::{Engine, Limits};
use decel_model::{CellAddr, SheetId};
use decel_store::Store;
use proptest::prelude::*;
use rusqlite::{Connection, params};

#[derive(Debug, Clone)]
enum Hostile {
    Cell(i64, i64, i64, Option<i64>, Vec<u8>, Option<Vec<u8>>),
    Meta(Vec<u8>, String),
    Sheet(i64, String, i64),
    Layout(i64, i64, i64, f64),
    Chart(i64, i64, String),
    Txn(i64, String, i64, Option<i64>),
    CellChange(i64, i64, i64, i64, Option<i64>, Vec<u8>),
    Other(i64, i64, String, Option<String>, Option<String>),
    Setting(String, String),
}

fn blob() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 0..20),
        Just(vec![0u8; 16]),
        Just(b"NaN".to_vec()),
        Just(b"12".to_vec()),
    ]
}

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("decel-meta/1\nvalue\thook\t9\nreturn 1\n\n".to_string()),
        Just("decel-meta/1\nvalue\thook\t999\nx".to_string()),
        Just("decel-meta/1\nend\tfield\t2\n1\n\n".to_string()),
        Just("decel-meta/1\nvalue\thook\t21\nwhile true do end\n\n".to_string()),
        Just(r#"{"sheet":1,"row":0,"col":0,"spec":"{}"}"#.to_string()),
        Just(r#"{"sheet":999,"row":4294967295,"col":1,"spec":"{\"series\":[{\"name\":\"x\",\"range\":{\"sheet\":1,\"rect\":{\"r1\":0,\"c1\":0,\"r2\":4294967295,\"c2\":4294967295}}}]}"}"#.to_string()),
        Just(r#"{"name":"S","position":99}"#.to_string()),
        Just(r#"{"op":"delete_rows","sheet":1,"at":0,"n":4294967295}"#.to_string()),
        Just("null".to_string()),
        Just("[]".to_string()),
        "\\PC{0,40}",
    ]
}

fn coord() -> impl Strategy<Value = i64> {
    prop_oneof![
        0i64..20,
        Just(-1),
        Just(i64::from(u32::MAX)),
        Just(i64::MAX)
    ]
}

fn hostile() -> impl Strategy<Value = Hostile> {
    prop_oneof![
        (
            coord(),
            coord(),
            coord(),
            proptest::option::of(-1i64..6),
            blob(),
            proptest::option::of(blob())
        )
            .prop_map(|(s, r, c, vt, v, m)| Hostile::Cell(s, r, c, vt, v, m)),
        (blob(), text()).prop_map(|(h, t)| Hostile::Meta(h, t)),
        (coord(), "\\PC{0,8}", coord()).prop_map(|(i, n, p)| Hostile::Sheet(i, n, p)),
        (coord(), -1i64..3, coord(), any::<f64>())
            .prop_map(|(s, a, i, z)| Hostile::Layout(s, a, i, z)),
        (coord(), coord(), text()).prop_map(|(i, s, t)| Hostile::Chart(i, s, t)),
        (
            1i64..30,
            "\\PC{0,8}",
            -1i64..6,
            proptest::option::of(-1i64..40)
        )
            .prop_map(|(i, l, k, t)| Hostile::Txn(i, l, k, t)),
        (
            1i64..30,
            coord(),
            coord(),
            coord(),
            proptest::option::of(-1i64..6),
            blob()
        )
            .prop_map(|(t, s, r, c, vt, v)| Hostile::CellChange(t, s, r, c, vt, v)),
        (
            1i64..30,
            0i64..5,
            prop_oneof![
                Just("sheet:1".to_string()),
                Just("chart:1".to_string()),
                Just("layout:1:0:3".to_string()),
                Just("structure".to_string()),
                "\\PC{0,12}"
            ],
            proptest::option::of(text()),
            proptest::option::of(text())
        )
            .prop_map(|(t, s, e, b, a)| Hostile::Other(t, s, e, b, a)),
        ("\\PC{0,12}", text()).prop_map(|(k, v)| Hostile::Setting(k, v)),
    ]
}

fn inject(conn: &Connection, h: &Hostile) {
    // Constraint violations are fine: the row just isn't inserted.
    let _ = match h {
        Hostile::Cell(s, r, c, vt, v, m) => conn.execute(
            "INSERT OR REPLACE INTO cells(sheet_id,row,col,vtype,value,meta) VALUES (?1,?2,?3,?4,?5,?6)",
            params![s, r, c, vt, v, m],
        ),
        Hostile::Meta(h, t) => conn.execute(
            "INSERT OR REPLACE INTO metatables(hash,canonical,created_txn) VALUES (?1,?2,1)",
            params![h, t],
        ),
        Hostile::Sheet(i, n, p) => conn.execute(
            "INSERT OR REPLACE INTO sheets(id,name,position) VALUES (?1,?2,?3)",
            params![i, n, p],
        ),
        Hostile::Layout(s, a, i, z) => conn.execute(
            "INSERT OR REPLACE INTO layout(sheet_id,axis,idx,size) VALUES (?1,?2,?3,?4)",
            params![s, a, i, z],
        ),
        Hostile::Chart(i, s, t) => conn.execute(
            "INSERT OR REPLACE INTO charts(id,sheet_id,spec) VALUES (?1,?2,?3)",
            params![i, s, t],
        ),
        Hostile::Txn(i, l, k, t) => conn.execute(
            "INSERT OR REPLACE INTO txns(id,ts,label,kind,target) VALUES (?1,0,?2,?3,?4)",
            params![i, l, k, t],
        ),
        Hostile::CellChange(t, s, r, c, vt, v) => conn.execute(
            "INSERT OR REPLACE INTO cell_changes(txn,sheet_id,row,col,b_vtype,b_value,b_meta,a_vtype,a_value,a_meta)
             VALUES (?1,?2,?3,?4,?5,?6,NULL,?5,?6,NULL)",
            params![t, s, r, c, vt, v],
        ),
        Hostile::Other(t, s, e, b, a) => conn.execute(
            "INSERT OR REPLACE INTO other_changes(txn,seq,entity,before,after) VALUES (?1,?2,?3,?4,?5)",
            params![t, s, e, b, a],
        ),
        Hostile::Setting(k, v) => conn.execute(
            "INSERT OR REPLACE INTO workbook(key,value) VALUES (?1,?2)",
            params![format!("settings.{k}"), v],
        ),
    };
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: if cfg!(debug_assertions) { 24 } else { 300 },
        ..ProptestConfig::default()
    })]

    #[test]
    fn hostile_rows_never_panic(rows in proptest::collection::vec(hostile(), 1..25)) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.decel");
        drop(Store::create(&path).unwrap());
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "foreign_keys", false).unwrap();
            for r in &rows {
                inject(&conn, r);
            }
        }
        let Ok(mut store) = Store::open(&path) else { return Ok(()) };
        let limits = Limits {
            instructions: 200_000,
            time: std::time::Duration::from_millis(50),
            memory: 32 * 1024 * 1024,
        };
        if let Ok(wb) = store.load()
            && let Ok(mut engine) = Engine::new(wb, limits)
        {
            engine.recalc_all();
            for s in [1u32, 2, 999] {
                for r in 0..3 {
                    let a = CellAddr::new(SheetId(s), r, 0);
                    let _ = engine.display(a);
                    let _ = engine.tooltip(a);
                    let _ = engine.edit_text(a);
                }
            }
        }
        for _ in 0..3 {
            let _ = store.undo();
        }
        let _ = store.redo();
        let _ = store.cell_history(CellAddr::new(SheetId(1), 0, 0));
        let _ = store.txns();
        let _ = store.load();
    }
}
