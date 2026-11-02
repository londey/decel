// SPDX-License-Identifier: MIT OR Apache-2.0

//! The engine thread (DESIGN §9.2).
//!
//! The actor owns the store and the engine. It handles commands one at a
//! time, recalculates in ~5 ms slices between commands (a new command
//! pre-empts the slice), and pushes display updates only for subscribed
//! rectangles. When idle it checkpoints the SQLite WAL.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError, unbounded};
use decel_model::{
    Axis, Cell, CellAddr, CellStorage, Change, Member, MemberKind, MetaHash, MetaSource, Op, Rect,
    SheetId, TxnInput, Workbook,
};
use decel_store::{Committed, Store, StoreError};

use crate::fill::{FillWhat, auto_fill_run, fill_from_edge, fill_pattern};
use crate::protocol::{
    ClipData, Command, Event, FillKind, MetaEditMode, MetaSummary, PasteMode, Reply, SheetInfo,
    WorkbookInfo,
};
use crate::{Engine, Limits};

/// Rows/columns sent beyond a subscribed rectangle, so small scrolls are
/// already cached.
pub const MARGIN: u32 = 20;

/// Length of one recalculation slice.
const SLICE: Duration = Duration::from_millis(5);

/// Idle time before checkpointing the WAL.
const IDLE_CHECKPOINT: Duration = Duration::from_millis(500);

/// Why a command failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("the file is open in another decel (pid {pid:?} on {host:?})")]
    Locked {
        pid: Option<u32>,
        host: Option<String>,
    },
    #[error("no workbook is open")]
    NoWorkbook,
    #[error("{0}")]
    Failed(String),
}

impl From<StoreError> for CommandError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Locked { pid, host } => CommandError::Locked { pid, host },
            other => CommandError::Failed(other.to_string()),
        }
    }
}

impl From<crate::EngineError> for CommandError {
    fn from(e: crate::EngineError) -> Self {
        CommandError::Failed(e.to_string())
    }
}

type Ack = Result<Reply, CommandError>;

/// The UI's handle to the engine thread.
pub struct EngineHandle {
    tx: Sender<(u64, Command)>,
    events: Receiver<Event>,
    next_seq: u64,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    /// Starts the engine thread with no workbook open.
    pub fn spawn(limits: Limits) -> std::io::Result<Self> {
        let (tx, rx) = unbounded();
        let (etx, events) = unbounded();
        let thread = std::thread::Builder::new()
            .name("decel-engine".into())
            .spawn(move || {
                Actor {
                    doc: None,
                    subs: BTreeMap::new(),
                    rx,
                    events: etx,
                    limits,
                    wal_dirty: false,
                }
                .run()
            })?;
        Ok(Self {
            tx,
            events,
            next_seq: 1,
            thread: Some(thread),
        })
    }

    /// Sends a command and returns its sequence number.
    pub fn send(&mut self, cmd: Command) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let _ = self.tx.send((seq, cmd));
        seq
    }

    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.tx.send((0, Command::Shutdown));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Doc {
    store: Store,
    engine: Engine,
    /// Colour index per metatable hash (DESIGN §8.3).
    colors: BTreeMap<MetaHash, u32>,
}

const COLORS_KEY: &str = "meta_colors";

impl Doc {
    /// Assigns colour indexes to hashes not seen before, in the given order.
    /// Returns whether anything was assigned (and persists it).
    fn assign_colors(&mut self, hashes: impl IntoIterator<Item = MetaHash>) -> bool {
        let mut next = self.colors.values().max().map_or(0, |m| m + 1);
        let mut changed = false;
        for h in hashes {
            if let std::collections::btree_map::Entry::Vacant(e) = self.colors.entry(h) {
                e.insert(next);
                next += 1;
                changed = true;
            }
        }
        if changed && !self.store.is_read_only() {
            let map: BTreeMap<String, u32> = self
                .colors
                .iter()
                .map(|(h, i)| (h.to_string(), *i))
                .collect();
            if let Ok(json) = serde_json::to_string(&map) {
                let _ = self.store.set_setting(COLORS_KEY, &json);
                self.engine.set_setting(COLORS_KEY, &json);
            }
        }
        changed
    }
}

fn parse_hash(s: &str) -> Option<MetaHash> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(MetaHash(out))
}

/// Hashes in first-appearance order: sheets in tab order, then row-major.
fn hashes_in_order(wb: &Workbook) -> Vec<MetaHash> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for sheet in wb.sheets_ordered() {
        for (_, cell) in sheet.cells.iter() {
            if let Some(h) = cell.meta
                && seen.insert(h)
            {
                out.push(h);
            }
        }
    }
    out
}

struct Actor {
    doc: Option<Doc>,
    subs: BTreeMap<u32, (SheetId, Rect)>,
    rx: Receiver<(u64, Command)>,
    events: Sender<Event>,
    limits: Limits,
    wal_dirty: bool,
}

impl Actor {
    fn run(mut self) {
        loop {
            let pending = self.doc.as_ref().map_or(0, |d| d.engine.pending());
            let msg = if pending > 0 {
                match self.rx.try_recv() {
                    Ok(m) => Some(m),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return self.shutdown(),
                }
            } else if self.wal_dirty {
                match self.rx.recv_timeout(IDLE_CHECKPOINT) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => {
                        self.checkpoint();
                        None
                    }
                    Err(RecvTimeoutError::Disconnected) => return self.shutdown(),
                }
            } else {
                match self.rx.recv() {
                    Ok(m) => Some(m),
                    Err(_) => return self.shutdown(),
                }
            };
            if let Some((seq, cmd)) = msg {
                if cmd == Command::Shutdown {
                    return self.shutdown();
                }
                let result = self.handle(cmd);
                self.emit(Event::Ack { seq, result });
                self.flush();
                continue;
            }
            if pending > 0 {
                let rx = self.rx.clone();
                let start = Instant::now();
                let left = match self.doc.as_mut() {
                    Some(doc) => doc
                        .engine
                        .recalc(|| start.elapsed() >= SLICE || !rx.is_empty()),
                    None => 0,
                };
                self.flush();
                self.emit(Event::RecalcProgress { pending: left });
            }
        }
    }

    fn shutdown(&mut self) {
        self.checkpoint();
    }

    fn checkpoint(&mut self) {
        if let Some(doc) = self.doc.as_mut() {
            let _ = doc.store.checkpoint();
        }
        self.wal_dirty = false;
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    fn doc(&mut self) -> Result<&mut Doc, CommandError> {
        self.doc.as_mut().ok_or(CommandError::NoWorkbook)
    }

    /// Pushes display updates for changed cells inside subscriptions.
    fn flush(&mut self) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let changed = doc.engine.take_changed();
        if changed.is_empty() || self.subs.is_empty() {
            return;
        }
        for (&sub, &(sheet, rect)) in &self.subs {
            let area = rect.expand(MARGIN);
            let mut hits: Vec<CellAddr> = changed
                .iter()
                .copied()
                .filter(|a| a.sheet == sheet && area.contains(a.row, a.col))
                .collect();
            if hits.is_empty() {
                continue;
            }
            hits.sort();
            let cells = hits
                .into_iter()
                .map(|a| (a, doc.engine.display(a)))
                .collect();
            let _ = self.events.send(Event::CellsUpdated {
                sub,
                sheet,
                rect: None,
                full: false,
                cells,
            });
        }
    }

    /// Sends every populated cell in a subscription.
    fn snapshot(&mut self, sub: u32) {
        let Some(&(sheet, rect)) = self.subs.get(&sub) else {
            return;
        };
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let area = rect.expand(MARGIN);
        let addrs: Vec<CellAddr> = doc
            .engine
            .workbook()
            .sheet(sheet)
            .map(|s| {
                s.cells
                    .iter_rect(area)
                    .map(|((r, c), _)| CellAddr::new(sheet, r, c))
                    .collect()
            })
            .unwrap_or_default();
        let cells = addrs
            .into_iter()
            .map(|a| (a, doc.engine.display(a)))
            .collect();
        self.emit(Event::CellsUpdated {
            sub,
            sheet,
            rect: Some(area),
            full: true,
            cells,
        });
    }

    fn workbook_info(&self) -> Option<WorkbookInfo> {
        let doc = self.doc.as_ref()?;
        let wb = doc.engine.workbook();
        Some(WorkbookInfo {
            path: doc.store.path().map(Path::to_path_buf),
            read_only: doc.store.is_read_only(),
            sheets: wb
                .sheets_ordered()
                .into_iter()
                .map(|s| SheetInfo {
                    id: s.id,
                    name: s.name.clone(),
                    col_widths: s.layout.cols.clone(),
                    row_heights: s.layout.rows.clone(),
                })
                .collect(),
            charts: wb.charts.clone(),
            settings: wb.settings.clone(),
            meta_colors: doc.colors.clone(),
        })
    }

    fn emit_workbook(&self) {
        if let Some(info) = self.workbook_info() {
            self.emit(Event::Workbook(Box::new(info)));
        }
    }

    fn load(&mut self, store: Store) -> Ack {
        let wb = store.load()?;
        let mut colors = BTreeMap::new();
        if let Some(json) = wb.settings.get(COLORS_KEY)
            && let Ok(map) = serde_json::from_str::<BTreeMap<String, u32>>(json)
        {
            colors = map
                .into_iter()
                .filter_map(|(k, v)| Some((parse_hash(&k)?, v)))
                .collect();
        }
        let order = hashes_in_order(&wb);
        let engine = Engine::new(wb, self.limits)?;
        let mut doc = Doc {
            store,
            engine,
            colors,
        };
        doc.assign_colors(order);
        self.doc = Some(doc);
        self.subs.clear();
        self.emit_workbook();
        self.emit_txn_state(String::new());
        Ok(Reply::Done)
    }

    fn emit_txn_state(&self, label: String) {
        if let Some(doc) = self.doc.as_ref() {
            self.emit(Event::TxnCommitted {
                label,
                can_undo: doc.store.can_undo(),
                can_redo: doc.store.can_redo(),
                undo_label: doc.store.undo_label().ok().flatten(),
                redo_label: doc.store.redo_label().ok().flatten(),
            });
        }
    }

    /// Feeds committed changes to the engine and announces them.
    fn committed(&mut self, c: Committed) -> Reply {
        let mut structural = false;
        let affected: Vec<CellAddr> = c
            .changes
            .iter()
            .filter_map(|ch| match ch {
                Change::Cell { addr, .. } => Some(*addr),
                _ => {
                    structural = true;
                    None
                }
            })
            .collect();
        let mut new_colors = false;
        if let Some(doc) = self.doc.as_mut() {
            doc.engine.apply(&c.changes);
            let mut cells: Vec<(CellAddr, MetaHash)> = c
                .changes
                .iter()
                .filter_map(|ch| match ch {
                    Change::Cell { addr, after, .. } => after.meta.map(|h| (*addr, h)),
                    _ => None,
                })
                .collect();
            cells.sort();
            new_colors = doc.assign_colors(cells.into_iter().map(|(_, h)| h));
        }
        self.wal_dirty = true;
        self.emit_txn_state(c.txn.label.clone());
        if new_colors && !structural {
            self.emit_workbook();
        }
        if structural {
            self.emit_workbook();
            // Structure ops shift cells: refresh every subscription.
            let subs: Vec<u32> = self.subs.keys().copied().collect();
            for s in subs {
                self.snapshot(s);
            }
        }
        Reply::Committed { affected }
    }

    fn commit(&mut self, input: TxnInput) -> Ack {
        let doc = self.doc()?;
        for m in &input.metas {
            doc.engine.add_meta(m)?;
        }
        match doc.store.apply(input)? {
            Some(c) => Ok(self.committed(c)),
            None => Ok(Reply::NoChange),
        }
    }

    fn set_cells(
        &mut self,
        label: String,
        cells: Vec<(CellAddr, Cell)>,
        metas: Vec<MetaSource>,
    ) -> Ack {
        if cells.is_empty() {
            return Ok(Reply::NoChange);
        }
        self.commit(TxnInput {
            label,
            ops: vec![Op::SetCells(cells)],
            metas,
        })
    }

    fn op(&mut self, label: &str, op: Op) -> Ack {
        self.commit(TxnInput::new(label).op(op))
    }

    fn handle(&mut self, cmd: Command) -> Ack {
        match cmd {
            Command::Open { path, read_only } => {
                let store = if read_only {
                    Store::open_read_only(&path)?
                } else {
                    Store::open(&path)?
                };
                self.load(store)
            }
            Command::New { path } => {
                let store = match path {
                    Some(p) => Store::create(&p)?,
                    None => Store::in_memory()?,
                };
                self.load(store)
            }
            Command::SaveAs { path } => {
                self.doc()?.store.save_as(&path)?;
                self.emit_workbook();
                Ok(Reply::Done)
            }
            Command::EnterText { addr, text } => {
                let cell = match self.doc()?.engine.enter_text(addr, &text) {
                    Ok(c) => c,
                    Err(rej) => return Ok(Reply::Rejected(rej.message)),
                };
                self.set_cells(format!("Edit {addr}"), vec![(addr, cell)], Vec::new())
            }
            Command::SetCells { label, cells } => self.set_cells(label, cells, Vec::new()),
            Command::SetMeta {
                targets,
                source,
                mode,
            } => self.set_meta(targets, source, mode),
            Command::Detach { targets } => self.detach(targets),
            Command::Fill {
                kind,
                sheet,
                src,
                dst,
                with_values,
                preview,
            } => {
                let what = if with_values {
                    FillWhat::MetaAndValues
                } else {
                    FillWhat::Meta
                };
                let doc = self.doc()?;
                let wb = doc.engine.workbook();
                let cells = match kind {
                    FillKind::Drag => fill_pattern(wb, sheet, src, dst, what),
                    FillKind::Down => fill_from_edge(wb, sheet, dst, Axis::Row, what),
                    FillKind::Right => fill_from_edge(wb, sheet, dst, Axis::Col, what),
                };
                if preview {
                    return Ok(Reply::Addresses(
                        cells.into_iter().map(|(a, _)| a).collect(),
                    ));
                }
                let label = match kind {
                    FillKind::Drag => format!("Fill {} cells", cells.len()),
                    FillKind::Down => format!("Fill down {} cells", cells.len()),
                    FillKind::Right => format!("Fill right {} cells", cells.len()),
                };
                self.set_cells(label, cells, Vec::new())
            }
            Command::AutoFill {
                addr,
                axis,
                preview,
            } => {
                let doc = self.doc()?;
                let wb = doc.engine.workbook();
                let run = auto_fill_run(wb, addr, axis);
                if preview {
                    return Ok(Reply::Addresses(run));
                }
                let meta = wb.cell(addr).and_then(|c| c.meta);
                let cells: Vec<(CellAddr, Cell)> = run
                    .into_iter()
                    .map(|a| {
                        let cur = wb.cell(a).cloned().unwrap_or_default();
                        (a, Cell { raw: cur.raw, meta })
                    })
                    .collect();
                let label = format!("Auto-fill {} cells", cells.len());
                self.set_cells(label, cells, Vec::new())
            }
            Command::Paste { at, clip, mode } => self.paste(at, clip, mode),
            Command::Copy { sheet, rect } => {
                Ok(Reply::Clip(copy(&mut self.doc()?.engine, sheet, rect)))
            }
            Command::PasteText { at, text } => {
                let engine = &mut self.doc()?.engine;
                let mut cells = Vec::new();
                let mut rejected = 0;
                for (r, line) in text.trim_end_matches(['\n', '\r']).split('\n').enumerate() {
                    for (c, field) in line.trim_end_matches('\r').split('\t').enumerate() {
                        let Some(a) = at.offset(r as i64, c as i64) else {
                            continue;
                        };
                        match engine.enter_text(a, field) {
                            Ok(cell) => cells.push((a, cell)),
                            Err(_) => rejected += 1,
                        }
                    }
                }
                let label = if rejected > 0 {
                    format!("Paste ({rejected} cell(s) rejected)")
                } else {
                    "Paste".into()
                };
                self.set_cells(label, cells, Vec::new())
            }
            Command::NewMeta { targets } => {
                let wb = self.doc()?.engine.workbook();
                let mut n = 1u32;
                let src = loop {
                    let src = MetaSource::default().with("kind", &format!("\"kind {n}\""));
                    if !wb.metas.contains_key(&src.hash()) {
                        break src;
                    }
                    n += 1;
                };
                self.set_meta(targets, Some(src), MetaEditMode::Cow)
            }
            Command::MetaSummary { sheet } => {
                let wb = self.doc()?.engine.workbook();
                let mut counts: HashMap<MetaHash, (usize, usize)> = HashMap::new();
                for s in wb.sheets.values() {
                    for (_, cell) in s.cells.iter() {
                        if let Some(h) = cell.meta {
                            let e = counts.entry(h).or_default();
                            e.1 += 1;
                            if s.id == sheet {
                                e.0 += 1;
                            }
                        }
                    }
                }
                let mut out: Vec<MetaSummary> = counts
                    .into_iter()
                    .filter(|(_, (n, _))| *n > 0)
                    .map(|(hash, (count, total))| {
                        let src = wb.meta(&hash);
                        MetaSummary {
                            hash,
                            kind: src.and_then(|s| s.kind_label()),
                            unit: src.and_then(|s| s.unit_label()),
                            count,
                            total,
                        }
                    })
                    .collect();
                out.sort_by(|a, b| b.count.cmp(&a.count).then(a.hash.cmp(&b.hash)));
                Ok(Reply::Summary(out))
            }
            Command::Undo => match self.doc()?.store.undo()? {
                Some(c) => Ok(self.committed(c)),
                None => Ok(Reply::NoChange),
            },
            Command::Redo => match self.doc()?.store.redo()? {
                Some(c) => Ok(self.committed(c)),
                None => Ok(Reply::NoChange),
            },
            Command::Restore {
                addr,
                version,
                version_addr,
            } => match self.doc()?.store.restore(addr, version, version_addr)? {
                Some(c) => Ok(self.committed(c)),
                None => Ok(Reply::NoChange),
            },
            Command::Subscribe { id, sheet, rect } => {
                self.subs.insert(id, (sheet, rect));
                self.snapshot(id);
                Ok(Reply::Done)
            }
            Command::Unsubscribe { id } => {
                self.subs.remove(&id);
                Ok(Reply::Done)
            }
            Command::Preview { addr, source } => {
                let p = self.doc()?.engine.preview(addr, &source)?;
                Ok(Reply::Preview(Box::new(p)))
            }
            Command::BeginEdit { addr } => {
                let engine = &mut self.doc()?.engine;
                Ok(Reply::Edit {
                    text: engine.edit_text(addr),
                    hint: engine.edit_hint(addr),
                })
            }
            Command::Tooltip { addr } => Ok(Reply::Tooltip(self.doc()?.engine.tooltip(addr))),
            Command::DataEdge { from, dr, dc } => {
                Ok(Reply::Addresses(vec![crate::nav::data_edge(
                    self.doc()?.engine.workbook(),
                    from,
                    dr,
                    dc,
                )]))
            }
            Command::Values { sheet, rect } => {
                let engine = &mut self.doc()?.engine;
                let addrs: Vec<CellAddr> = engine
                    .workbook()
                    .sheet(sheet)
                    .map(|s| {
                        s.cells
                            .iter_rect(rect)
                            .map(|((r, c), _)| CellAddr::new(sheet, r, c))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Reply::Values(
                    addrs
                        .into_iter()
                        .map(|a| (a, engine.final_value(a)))
                        .collect(),
                ))
            }
            Command::QueryHistory { addr } => {
                Ok(Reply::History(self.doc()?.store.cell_history(addr)?))
            }
            Command::QueryShareSet { hash } => {
                Ok(Reply::ShareSet(self.doc()?.store.share_set(&hash)?))
            }
            Command::QueryMeta { hash } => Ok(Reply::Meta(
                self.doc()?
                    .engine
                    .workbook()
                    .meta(&hash)
                    .map(|m| (**m).clone()),
            )),
            Command::QueryTxns { limit } => {
                let store = &self.doc()?.store;
                let mut txns = store.txns()?;
                let skip = txns.len().saturating_sub(limit);
                txns.drain(..skip);
                Ok(Reply::Txns {
                    txns,
                    undo: store.undo_stack().to_vec(),
                    redo: store.redo_stack().to_vec(),
                })
            }
            Command::AddSheet { name } => self.op(
                &format!("Add sheet {name}"),
                Op::AddSheet {
                    name,
                    position: None,
                },
            ),
            Command::RenameSheet { id, name } => self.op(
                &format!("Rename sheet to {name}"),
                Op::RenameSheet { id, name },
            ),
            Command::MoveSheet { id, position } => {
                self.op("Move sheet", Op::MoveSheet { id, position })
            }
            Command::DeleteSheet { id } => self.op("Delete sheet", Op::DeleteSheet { id }),
            Command::SetLayout {
                sheet,
                axis,
                idx,
                size,
            } => {
                let label = match axis {
                    Axis::Col => "Resize column",
                    Axis::Row => "Resize row",
                };
                self.op(
                    label,
                    Op::SetLayout {
                        sheet,
                        axis,
                        idx,
                        size,
                    },
                )
            }
            Command::Structure(op) => {
                let (_, n) = op.span();
                let label = match op {
                    decel_model::StructOp::InsertRows { .. } => format!("Insert {n} row(s)"),
                    decel_model::StructOp::DeleteRows { .. } => format!("Delete {n} row(s)"),
                    decel_model::StructOp::InsertCols { .. } => format!("Insert {n} column(s)"),
                    decel_model::StructOp::DeleteCols { .. } => format!("Delete {n} column(s)"),
                };
                self.op(&label, Op::Structure(op))
            }
            Command::ChartUpsert { id, chart } => {
                let label = if id.is_some() {
                    "Edit chart"
                } else {
                    "Insert chart"
                };
                self.op(label, Op::ChartUpsert { id, chart })
            }
            Command::ChartDelete { id } => self.op("Delete chart", Op::ChartDelete { id }),
            Command::SetSetting { key, value } => {
                let doc = self.doc()?;
                doc.store.set_setting(&key, &value)?;
                doc.engine.set_setting(&key, &value);
                Ok(Reply::Done)
            }
            Command::Shutdown => Ok(Reply::Done),
        }
    }

    fn set_meta(
        &mut self,
        targets: Vec<CellAddr>,
        source: Option<MetaSource>,
        mode: MetaEditMode,
    ) -> Ack {
        let doc = self.doc()?;
        let mut cells: Vec<CellAddr> = targets.clone();
        if mode == MetaEditMode::Shared
            && let Some(h) = targets
                .first()
                .and_then(|a| doc.engine.workbook().cell(*a))
                .and_then(|c| c.meta)
        {
            cells.extend(doc.store.share_set(&h)?);
        }
        let mut seen = HashSet::new();
        cells.retain(|a| seen.insert(*a));
        let hash = source.as_ref().map(MetaSource::hash);
        let wb = doc.engine.workbook();
        let updates: Vec<(CellAddr, Cell)> = cells
            .into_iter()
            .map(|a| {
                let cur = wb.cell(a).cloned().unwrap_or_default();
                (
                    a,
                    Cell {
                        raw: cur.raw,
                        meta: hash,
                    },
                )
            })
            .collect();
        let label = match (&source, updates.len()) {
            (None, n) => format!("Remove metatable from {n} cell(s)"),
            (Some(s), 1) => format!(
                "Edit {}",
                s.kind_label().unwrap_or_else(|| "metatable".into())
            ),
            (Some(s), n) => format!(
                "Edit {} ({n} cells)",
                s.kind_label().unwrap_or_else(|| "metatable".into())
            ),
        };
        self.set_cells(label, updates, source.into_iter().collect())
    }

    fn detach(&mut self, targets: Vec<CellAddr>) -> Ack {
        let doc = self.doc()?;
        let wb = doc.engine.workbook();
        let mut groups: HashMap<MetaHash, Vec<CellAddr>> = HashMap::new();
        for a in targets {
            if let Some(h) = wb.cell(a).and_then(|c| c.meta) {
                groups.entry(h).or_default().push(a);
            }
        }
        let mut cells = Vec::new();
        let mut metas = Vec::new();
        for (h, addrs) in groups {
            let Some(src) = wb.meta(&h) else { continue };
            let copy = private_copy(wb, src);
            let nh = copy.hash();
            for a in addrs {
                let cur = wb.cell(a).cloned().unwrap_or_default();
                cells.push((
                    a,
                    Cell {
                        raw: cur.raw,
                        meta: Some(nh),
                    },
                ));
            }
            metas.push(copy);
        }
        let label = format!("Detach {} cell(s)", cells.len());
        self.set_cells(label, cells, metas)
    }

    fn paste(&mut self, at: CellAddr, clip: ClipData, mode: PasteMode) -> Ack {
        let doc = self.doc()?;
        let wb = doc.engine.workbook();
        let cols = clip.cols.max(1);
        let mut cells = Vec::with_capacity(clip.cells.len());
        for (i, src) in clip.cells.iter().enumerate() {
            let i = i as u32;
            let Some(a) = at.offset(i64::from(i / cols), i64::from(i % cols)) else {
                continue;
            };
            let cur = wb.cell(a).cloned().unwrap_or_default();
            let cell = match mode {
                PasteMode::All => src.clone(),
                PasteMode::Values => Cell {
                    raw: src.raw.clone(),
                    meta: cur.meta,
                },
                PasteMode::Meta => Cell {
                    raw: cur.raw,
                    meta: src.meta,
                },
            };
            cells.push((a, cell));
        }
        let used: HashSet<MetaHash> = cells.iter().filter_map(|(_, c)| c.meta).collect();
        let metas = clip
            .metas
            .into_iter()
            .filter(|m| used.contains(&m.hash()))
            .collect();
        let label = match mode {
            PasteMode::All => "Paste",
            PasteMode::Values => "Paste values",
            PasteMode::Meta => "Paste metatables",
        };
        self.set_cells(label.into(), cells, metas)
    }
}

/// Copies a rectangle with the metatable sources it references and its
/// edit text as TSV.
fn copy(engine: &mut Engine, sheet: SheetId, rect: Rect) -> ClipData {
    let mut cells = Vec::new();
    let mut metas = Vec::new();
    let mut seen = HashSet::new();
    let mut text = String::new();
    for (r, c) in rect.iter() {
        let addr = CellAddr::new(sheet, r, c);
        if c == rect.c1 && r > rect.r1 {
            text.push('\n');
        } else if c > rect.c1 {
            text.push('\t');
        }
        let edit = engine.edit_text(addr);
        text.push_str(&edit.replace(['\t', '\n'], " "));
        let wb = engine.workbook();
        let cell = wb.cell(addr).cloned().unwrap_or_default();
        if let Some(h) = cell.meta
            && seen.insert(h)
            && let Some(src) = wb.meta(&h)
        {
            metas.push((**src).clone());
        }
        cells.push(cell);
    }
    ClipData {
        rows: rect.r2 - rect.r1 + 1,
        cols: rect.c2 - rect.c1 + 1,
        cells,
        metas,
        text,
    }
}

/// A copy of `src` with a distinct hash, made by setting the `_copy` helper
/// to the first number that gives an unused hash.
fn private_copy(wb: &Workbook, src: &MetaSource) -> MetaSource {
    let mut n = 1u64;
    loop {
        let mut copy = src.clone();
        copy.set(Member {
            name: "_copy".into(),
            kind: MemberKind::Helper,
            source: n.to_string(),
        });
        if !wb.metas.contains_key(&copy.hash()) {
            return copy;
        }
        n += 1;
    }
}
