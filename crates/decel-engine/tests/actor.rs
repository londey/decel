// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Phase 3 acceptance tests: drive the engine thread only through channels.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use decel_engine::protocol::{Command, Event, FillKind, MetaEditMode, Reply};
use decel_engine::{CommandError, DisplayCell, EngineHandle, Limits};
use decel_model::{Cell, CellAddr, MetaSource, Op, Rect, SheetId, TxnInput, Value};
use decel_store::Store;

const S1: SheetId = SheetId(1);

fn a(row: u32, col: u32) -> CellAddr {
    CellAddr::new(S1, row, col)
}

/// A test client: tracks displayed cells from `CellsUpdated` events.
struct Client {
    h: EngineHandle,
    cells: HashMap<CellAddr, DisplayCell>,
    can_undo: bool,
    sheets: Vec<(SheetId, String)>,
}

impl Client {
    fn new() -> Self {
        Self {
            h: EngineHandle::spawn(Limits::default()).unwrap(),
            cells: HashMap::new(),
            can_undo: false,
            sheets: Vec::new(),
        }
    }

    fn absorb(&mut self, e: Event) -> Option<(u64, Result<Reply, CommandError>)> {
        match e {
            Event::Ack { seq, result } => return Some((seq, result)),
            Event::CellsUpdated {
                sheet,
                rect,
                full,
                cells,
                ..
            } => {
                if full && let Some(r) = rect {
                    self.cells
                        .retain(|a, _| a.sheet != sheet || !r.contains(a.row, a.col));
                }
                for (a, d) in cells {
                    self.cells.insert(a, d);
                }
            }
            Event::TxnCommitted { can_undo, .. } => self.can_undo = can_undo,
            Event::Workbook(info) => {
                self.sheets = info.sheets.iter().map(|s| (s.id, s.name.clone())).collect();
            }
            Event::RecalcProgress { .. } | Event::Error { .. } => {}
        }
        None
    }

    fn call(&mut self, cmd: Command) -> Result<Reply, CommandError> {
        let seq = self.h.send(cmd);
        loop {
            let e = self
                .h
                .events()
                .recv_timeout(Duration::from_secs(30))
                .expect("engine did not reply");
            if let Some((s, r)) = self.absorb(e)
                && s == seq
            {
                return r;
            }
        }
    }

    /// Absorbs events until the engine has been quiet for a while.
    fn settle(&mut self) {
        while let Ok(e) = self.h.events().recv_timeout(Duration::from_millis(300)) {
            self.absorb(e);
        }
    }

    fn text(&self, a: CellAddr) -> String {
        self.cells
            .get(&a)
            .map(|d| d.text.clone())
            .unwrap_or_default()
    }
}

fn running_total() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"running total\"")
        .with(
            "value",
            "local prev = c:up()\nlocal base = (prev.row >= 1 and prev.kind == self.kind) and prev.value or 0\nreturn base + c:left().value",
        )
}

#[test]
fn session_through_channels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.decel");
    let mut cl = Client::new();
    cl.call(Command::New {
        path: Some(path.clone()),
    })
    .unwrap();
    cl.call(Command::Subscribe {
        id: 1,
        sheet: S1,
        rect: Rect::new(0, 0, 499, 2),
    })
    .unwrap();

    // Values 1..=500 in column A.
    let cells = (0..500)
        .map(|r| (a(r, 0), Cell::value(Value::Int(1))))
        .collect();
    cl.call(Command::SetCells {
        label: "values".into(),
        cells,
    })
    .unwrap();
    // Typed edit with the default parser.
    assert!(matches!(
        cl.call(Command::EnterText {
            addr: a(0, 0),
            text: "2".into()
        })
        .unwrap(),
        Reply::Committed { .. }
    ));
    // Running total in B1, then fill down 500.
    cl.call(Command::SetMeta {
        targets: vec![a(0, 1)],
        source: Some(running_total()),
        mode: MetaEditMode::Cow,
    })
    .unwrap();
    let reply = cl
        .call(Command::Fill {
            kind: FillKind::Down,
            sheet: S1,
            src: Rect::cell(0, 1),
            dst: Rect::new(0, 1, 499, 1),
            with_values: false,
            preview: false,
        })
        .unwrap();
    assert!(matches!(reply, Reply::Committed { ref affected } if affected.len() == 499));
    cl.settle();
    assert_eq!(cl.text(a(0, 1)), "2");
    assert_eq!(cl.text(a(499, 1)), "501");

    // Undo the fill: only B1 keeps the meta.
    cl.call(Command::Undo).unwrap();
    cl.settle();
    assert_eq!(cl.text(a(0, 1)), "2");
    assert_eq!(cl.text(a(499, 1)), "");
    cl.call(Command::Redo).unwrap();
    cl.settle();
    assert_eq!(cl.text(a(499, 1)), "501");

    // Rejection via validate.
    cl.call(Command::SetMeta {
        targets: vec![a(0, 2)],
        source: Some(MetaSource::default().with(
            "validate",
            "if v > 10 then return false, 'max 10' end return true",
        )),
        mode: MetaEditMode::Cow,
    })
    .unwrap();
    assert_eq!(
        cl.call(Command::EnterText {
            addr: a(0, 2),
            text: "11".into()
        })
        .unwrap(),
        Reply::Rejected("max 10".into())
    );

    // Reopen in a new engine: state and undo survive.
    drop(cl);
    let mut cl = Client::new();
    assert!(matches!(
        cl.call(Command::Open {
            path: path.clone(),
            read_only: false
        }),
        Ok(Reply::Done)
    ));
    assert!(cl.can_undo);
    cl.call(Command::Subscribe {
        id: 1,
        sheet: S1,
        rect: Rect::new(0, 0, 499, 2),
    })
    .unwrap();
    cl.settle();
    assert_eq!(cl.text(a(499, 1)), "501");

    // A second engine can't take the lock, but can open read-only.
    let mut other = Client::new();
    assert!(matches!(
        other.call(Command::Open {
            path: path.clone(),
            read_only: false
        }),
        Err(CommandError::Locked { .. })
    ));
    other
        .call(Command::Open {
            path,
            read_only: true,
        })
        .unwrap();
}

#[test]
fn edits_preempt_long_recalc() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.decel");
    {
        let mut store = Store::create(&path).unwrap();
        let m = MetaSource::default().with("value", "return c:left().value * 2");
        let h = m.hash();
        let mut cells = Vec::with_capacity(200_000);
        for r in 0..100_000 {
            cells.push((a(r, 0), Cell::value(Value::Int(i64::from(r)))));
            cells.push((a(r, 1), Cell::EMPTY.with_meta(h)));
        }
        store
            .apply(TxnInput::new("big").op(Op::SetCells(cells)).meta(m))
            .unwrap();
    }
    let mut cl = Client::new();
    cl.call(Command::Open {
        path,
        read_only: false,
    })
    .unwrap();
    // Wait until recalculation is under way.
    loop {
        match cl.h.events().recv_timeout(Duration::from_secs(10)).unwrap() {
            Event::RecalcProgress { pending } if pending > 0 => break,
            e => {
                cl.absorb(e);
            }
        }
    }
    let start = Instant::now();
    let seq = cl.h.send(Command::EnterText {
        addr: a(0, 5),
        text: "7".into(),
    });
    let mut pending_after = 0;
    loop {
        match cl.h.events().recv_timeout(Duration::from_secs(10)).unwrap() {
            Event::Ack { seq: s, result } if s == seq => {
                assert!(result.is_ok());
                break;
            }
            Event::RecalcProgress { pending } => pending_after = pending,
            _ => {}
        }
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_millis(20), "ack took {elapsed:?}");
    assert!(pending_after > 0, "recalc should still have been running");
}

#[test]
fn cow_and_shared_meta_edits() {
    let mut cl = Client::new();
    cl.call(Command::New { path: None }).unwrap();
    cl.call(Command::AddSheet {
        name: "Other".into(),
    })
    .unwrap();
    let other = cl.sheets.iter().find(|(_, n)| n == "Other").unwrap().0;
    let m = MetaSource::default().with("kind", "\"shared\"");
    let h = m.hash();
    let mut targets: Vec<CellAddr> = (0..5).map(|r| a(r, 0)).collect();
    targets.extend((0..3).map(|r| CellAddr::new(other, r, 0)));
    cl.call(Command::SetMeta {
        targets: targets.clone(),
        source: Some(m.clone()),
        mode: MetaEditMode::Cow,
    })
    .unwrap();
    assert!(matches!(
        cl.call(Command::QueryShareSet { hash: h }).unwrap(),
        Reply::ShareSet(s) if s.len() == 8
    ));

    // COW: exactly one cell changes hash.
    let edited = m.clone().with("unit", "\"m\"");
    let reply = cl
        .call(Command::SetMeta {
            targets: vec![a(2, 0)],
            source: Some(edited.clone()),
            mode: MetaEditMode::Cow,
        })
        .unwrap();
    assert!(matches!(reply, Reply::Committed { ref affected } if affected == &[a(2, 0)]));
    assert!(matches!(
        cl.call(Command::QueryShareSet { hash: h }).unwrap(),
        Reply::ShareSet(s) if s.len() == 7
    ));

    // Shared: every remaining cell in the set, across sheets, in one txn.
    let shared = m.clone().with("unit", "\"km\"");
    let reply = cl
        .call(Command::SetMeta {
            targets: vec![a(0, 0)],
            source: Some(shared.clone()),
            mode: MetaEditMode::Shared,
        })
        .unwrap();
    match reply {
        Reply::Committed { affected } => {
            assert_eq!(affected.len(), 7);
            assert_eq!(affected.iter().filter(|x| x.sheet == other).count(), 3);
        }
        r => panic!("{r:?}"),
    }
    assert!(matches!(
        cl.call(Command::QueryShareSet { hash: shared.hash() }).unwrap(),
        Reply::ShareSet(s) if s.len() == 7
    ));
    // Undone as one step.
    cl.call(Command::Undo).unwrap();
    assert!(matches!(
        cl.call(Command::QueryShareSet { hash: h }).unwrap(),
        Reply::ShareSet(s) if s.len() == 7
    ));

    // Detach forks onto a private copy with a new hash.
    cl.call(Command::Detach {
        targets: vec![a(0, 0), a(1, 0)],
    })
    .unwrap();
    assert!(matches!(
        cl.call(Command::QueryShareSet { hash: h }).unwrap(),
        Reply::ShareSet(s) if s.len() == 5
    ));
}

#[test]
fn auto_fill_preview_then_confirm() {
    let mut cl = Client::new();
    cl.call(Command::New { path: None }).unwrap();
    let cells = (1..6)
        .map(|r| (a(r, 0), Cell::value(Value::Int(1))))
        .collect();
    cl.call(Command::SetCells {
        label: "v".into(),
        cells,
    })
    .unwrap();
    cl.call(Command::SetMeta {
        targets: vec![a(0, 0)],
        source: Some(MetaSource::default().with("kind", "\"k\"")),
        mode: MetaEditMode::Cow,
    })
    .unwrap();
    let preview = cl
        .call(Command::AutoFill {
            addr: a(0, 0),
            axis: decel_model::Axis::Row,
            preview: true,
        })
        .unwrap();
    let Reply::Addresses(run) = preview else {
        panic!()
    };
    assert_eq!(run.len(), 5);
    let reply = cl
        .call(Command::AutoFill {
            addr: a(0, 0),
            axis: decel_model::Axis::Row,
            preview: false,
        })
        .unwrap();
    assert!(matches!(reply, Reply::Committed { affected } if affected == run));
}

#[test]
fn copy_paste_values_and_meta() {
    let mut cl = Client::new();
    cl.call(Command::New { path: None }).unwrap();
    cl.call(Command::SetCells {
        label: "v".into(),
        cells: vec![(a(0, 0), Cell::value(Value::Int(5)))],
    })
    .unwrap();
    cl.call(Command::SetMeta {
        targets: vec![a(0, 0)],
        source: Some(MetaSource::default().with("display", "return 'x' .. v")),
        mode: MetaEditMode::Cow,
    })
    .unwrap();
    let Reply::Clip(clip) = cl
        .call(Command::Copy {
            sheet: S1,
            rect: Rect::cell(0, 0),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(clip.metas.len(), 1);
    cl.call(Command::Subscribe {
        id: 1,
        sheet: S1,
        rect: Rect::new(0, 0, 5, 5),
    })
    .unwrap();
    for (col, mode) in [
        (1, decel_engine::protocol::PasteMode::All),
        (2, decel_engine::protocol::PasteMode::Values),
    ] {
        cl.call(Command::Paste {
            at: a(0, col),
            clip: clip.clone(),
            mode,
        })
        .unwrap();
    }
    cl.settle();
    assert_eq!(cl.text(a(0, 1)), "x5");
    assert_eq!(cl.text(a(0, 2)), "5");
}

#[test]
fn colors_summary_new_meta_and_text_paste() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.decel");
    let mut cl = Client::new();
    cl.call(Command::New {
        path: Some(path.clone()),
    })
    .unwrap();
    let mut colors = Default::default();
    let a_src = MetaSource::default().with("kind", "\"a\"");
    let b_src = MetaSource::default().with("kind", "\"b\"");
    for (src, r) in [(&a_src, 0), (&b_src, 1)] {
        cl.call(Command::SetMeta {
            targets: vec![a(r, 0), a(r, 1)],
            source: Some(src.clone()),
            mode: MetaEditMode::Cow,
        })
        .unwrap();
    }
    // Workbook events carry first-appearance colour indexes.
    let seq = cl.h.send(Command::QueryTxns { limit: 1 });
    loop {
        match cl.h.events().recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Workbook(info) => colors = info.meta_colors.clone(),
            Event::Ack { seq: s, .. } if s == seq => break,
            _ => {}
        }
    }
    let _ = &colors;
    let Reply::Summary(sum) = cl.call(Command::MetaSummary { sheet: S1 }).unwrap() else {
        panic!()
    };
    assert_eq!(sum.len(), 2);
    assert!(sum.iter().all(|s| s.count == 2 && s.total == 2));
    assert_eq!(
        sum.iter()
            .find(|s| s.hash == a_src.hash())
            .unwrap()
            .kind
            .as_deref(),
        Some("a")
    );

    // New metatable on a selection: one shared, new hash.
    let Reply::Committed { affected } = cl
        .call(Command::NewMeta {
            targets: vec![a(5, 0), a(6, 0)],
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(affected.len(), 2);

    // Pasted text goes through the target's parse/validate hooks.
    cl.call(Command::SetMeta {
        targets: vec![a(3, 0), a(3, 1)],
        source: Some(
            MetaSource::default()
                .with("parse", "return tonumber(text) and tonumber(text) * 100")
                .with(
                    "validate",
                    "if v > 1000 then return false, 'big' end return true",
                ),
        ),
        mode: MetaEditMode::Cow,
    })
    .unwrap();
    let r = cl
        .call(Command::PasteText {
            at: a(3, 0),
            text: "2\t50\r\n".into(),
        })
        .unwrap();
    assert!(matches!(r, Reply::Committed { ref affected } if affected == &[a(3, 0)]));
    let Reply::Values(vals) = cl
        .call(Command::Values {
            sheet: S1,
            rect: Rect::new(3, 0, 3, 1),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(vals[0], (a(3, 0), Value::Int(200)));

    // Copy carries TSV edit text.
    let Reply::Clip(clip) = cl
        .call(Command::Copy {
            sheet: S1,
            rect: Rect::new(3, 0, 3, 1),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(clip.text, "200\t");

    // Colours persist across reopen.
    drop(cl);
    let mut cl = Client::new();
    let seq = cl.h.send(Command::Open {
        path,
        read_only: false,
    });
    let mut reopened = None;
    loop {
        match cl.h.events().recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Workbook(info) => reopened = Some(info.meta_colors.clone()),
            Event::Ack { seq: s, .. } if s == seq => break,
            _ => {}
        }
    }
    let reopened = reopened.unwrap();
    assert_eq!(reopened[&a_src.hash()], 0);
    assert_eq!(reopened[&b_src.hash()], 1);
    assert_eq!(reopened.len(), 4);
}

#[test]
fn csv_raw_round_trip_into_new_sheet() {
    use decel_engine::protocol::CsvTarget;
    let dir = tempfile::tempdir().unwrap();
    let mut cl = Client::new();
    cl.call(Command::New { path: None }).unwrap();
    let values = [
        Value::Int(-42),
        Value::Int(i64::MAX),
        Value::Num(0.1),
        Value::Num(3.0),
        Value::Num(-2.5e-300),
        Value::Bool(true),
        Value::str("plain"),
        Value::str("with, comma"),
        Value::str("with \"quotes\""),
        Value::str("multi\nline"),
        Value::str("42"),
        Value::str("'leading quote"),
    ];
    let cells = values
        .iter()
        .enumerate()
        .map(|(i, v)| (a(i as u32 / 3, i as u32 % 3), Cell::value(v.clone())))
        .collect();
    cl.call(Command::SetCells {
        label: "v".into(),
        cells,
    })
    .unwrap();
    let path = dir.path().join("out.csv");
    cl.call(Command::ExportCsv {
        path: path.clone(),
        sheet: S1,
        rect: None,
        raw: true,
        options: Default::default(),
    })
    .unwrap();
    let Reply::Imported {
        sheet: Some(new),
        rows,
        rejected,
    } = cl
        .call(Command::ImportCsv {
            path,
            target: CsvTarget::NewSheet,
            options: Default::default(),
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!((rows, rejected), (4, 0));
    let Reply::Values(got) = cl
        .call(Command::Values {
            sheet: new,
            rect: Rect::new(0, 0, 10, 10),
        })
        .unwrap()
    else {
        panic!()
    };
    let got: Vec<Value> = got.into_iter().map(|(_, v)| v).collect();
    assert_eq!(got, values);
    // One step to undo the whole import (sheet included).
    cl.call(Command::Undo).unwrap();
    assert!(!cl.sheets.iter().any(|(id, _)| *id == new));
}

#[test]
fn csv_import_at_cell_uses_parse_hooks() {
    use decel_engine::protocol::CsvTarget;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("in.csv");
    std::fs::write(&path, "h1;h2\n2;3\n500;4\n").unwrap();
    let mut cl = Client::new();
    cl.call(Command::New { path: None }).unwrap();
    cl.call(Command::SetMeta {
        targets: (1..3).map(|r| a(r, 0)).collect(),
        source: Some(
            MetaSource::default()
                .with("parse", "return tonumber(text) * 10")
                .with(
                    "validate",
                    "if v > 100 then return false, 'big' end return true",
                ),
        ),
        mode: MetaEditMode::Cow,
    })
    .unwrap();
    let r = cl
        .call(Command::ImportCsv {
            path,
            target: CsvTarget::At(a(0, 0)),
            options: decel_io::ImportOptions {
                header: true,
                ..Default::default()
            },
        })
        .unwrap();
    assert!(matches!(
        r,
        Reply::Imported {
            sheet: None,
            rows: 2,
            rejected: 1
        }
    ));
    let Reply::Values(got) = cl
        .call(Command::Values {
            sheet: S1,
            rect: Rect::new(0, 0, 2, 1),
        })
        .unwrap()
    else {
        panic!()
    };
    let map: HashMap<CellAddr, Value> = got.into_iter().collect();
    assert_eq!(map[&a(0, 0)], Value::str("h1"));
    assert_eq!(map[&a(1, 0)], Value::Int(20));
    assert_eq!(map[&a(1, 1)], Value::Int(3));
    // 500 was rejected by validate: the cell keeps its metatable and no value.
    assert_eq!(map[&a(2, 0)], Value::Nil);
}

/// 100 k rows must import in under 2 s and undo in one step (run in release:
/// `cargo nextest run --release -p decel-engine --run-ignored all csv_import_100k`).
#[test]
#[ignore]
fn csv_import_100k_rows() {
    use decel_engine::protocol::CsvTarget;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.csv");
    let mut text = String::with_capacity(4_000_000);
    for r in 0..100_000 {
        text.push_str(&format!("{r},{}.5,item {r}\n", r * 2));
    }
    std::fs::write(&path, text).unwrap();
    let mut cl = Client::new();
    cl.call(Command::New {
        path: Some(dir.path().join("big.decel")),
    })
    .unwrap();
    let start = Instant::now();
    let r = cl
        .call(Command::ImportCsv {
            path,
            target: CsvTarget::NewSheet,
            options: Default::default(),
        })
        .unwrap();
    let took = start.elapsed();
    assert!(matches!(r, Reply::Imported { rows: 100_000, .. }));
    assert!(took < Duration::from_secs(2), "import took {took:?}");
    let start = Instant::now();
    cl.call(Command::Undo).unwrap();
    eprintln!("import {took:?}, undo {:?}", start.elapsed());
}
