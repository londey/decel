// SPDX-License-Identifier: MIT OR Apache-2.0

//! SQLite-backed workbook store: schema, transactions, undo/redo and history.
//!
//! Every mutation is one SQLite transaction that updates the materialised
//! state tables and appends before/after images to the history tables
//! (DESIGN §7). Nothing in the history is ever deleted.

mod codec;
mod lock;
mod schema;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use decel_model::{
    Axis, Cell, CellAddr, CellStorage, Change, ChartId, ChartRecord, MetaHash, MetaSource, Op,
    Sheet, SheetId, SheetRecord, StructOp, TxnInput, Workbook, invert_changes,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};

use crate::codec::{OtherRecord, decode_cell, decode_other, encode_meta, encode_other, encode_raw};
use crate::lock::LockFile;

pub use crate::schema::{APPLICATION_ID, CURRENT_VERSION};

pub type TxnId = i64;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the file is open in another decel (pid {pid:?} on {host:?})")]
    Locked {
        pid: Option<u32>,
        host: Option<String>,
    },
    #[error("not a decel workbook")]
    NotADecelFile,
    #[error("workbook schema version {version} is newer than this decel supports ({supported})")]
    TooNew { version: i64, supported: i64 },
    #[error("the workbook is open read-only")]
    ReadOnly,
    #[error("a file already exists at {0}")]
    Exists(PathBuf),
    #[error("no sheet with id {0}")]
    NoSuchSheet(SheetId),
    #[error("no chart with id {0:?}")]
    NoSuchChart(ChartId),
    #[error("invalid sheet name {0:?}")]
    BadSheetName(String),
    #[error("a sheet named {0:?} already exists")]
    DuplicateSheetName(String),
    #[error("sheet id {0} is already used")]
    SheetIdInUse(SheetId),
    #[error("a workbook must keep at least one sheet")]
    LastSheet,
    #[error("unknown metatable {0}")]
    UnknownMeta(MetaHash),
    #[error("raw values must be bool, int, number or string")]
    InvalidRaw,
    #[error("invalid layout size")]
    InvalidSize,
    #[error("a structural operation must be the only op in its transaction")]
    StructureNotAlone,
    #[error("no history entry for that cell at txn {0}")]
    NoSuchVersion(TxnId),
    #[error("corrupt workbook: {0}")]
    Corrupt(&'static str),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<decel_model::MetaParseError> for StoreError {
    fn from(_: decel_model::MetaParseError) -> Self {
        StoreError::Corrupt("bad metatable source")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnKind {
    Normal,
    Undo,
    Redo,
    Restore,
}

impl TxnKind {
    fn to_int(self) -> i64 {
        match self {
            TxnKind::Normal => 0,
            TxnKind::Undo => 1,
            TxnKind::Redo => 2,
            TxnKind::Restore => 3,
        }
    }

    fn from_int(v: i64) -> Result<Self, StoreError> {
        Ok(match v {
            0 => TxnKind::Normal,
            1 => TxnKind::Undo,
            2 => TxnKind::Redo,
            3 => TxnKind::Restore,
            _ => return Err(StoreError::Corrupt("bad txn kind")),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TxnInfo {
    pub id: TxnId,
    /// Unix milliseconds.
    pub ts: i64,
    pub label: String,
    pub kind: TxnKind,
    pub target: Option<TxnId>,
}

/// A committed transaction and the changes it made, in application order.
#[derive(Debug, Clone, PartialEq)]
pub struct Committed {
    pub txn: TxnInfo,
    pub changes: Vec<Change>,
}

/// One version of a cell, from its history.
#[derive(Debug, Clone, PartialEq)]
pub struct CellVersion {
    pub txn: TxnInfo,
    /// The cell's address as of that transaction.
    pub addr: CellAddr,
    pub before: Cell,
    pub after: Cell,
}

const DEFAULT_SHEET: &str = "Sheet1";
const KEY_NEXT_SHEET: &str = "next_sheet_id";
const KEY_NEXT_CHART: &str = "next_chart_id";
const SETTINGS_PREFIX: &str = "settings.";

pub struct Store {
    conn: Connection,
    path: Option<PathBuf>,
    read_only: bool,
    undo: Vec<TxnId>,
    redo: Vec<TxnId>,
    /// Changes of recently committed txns, so undo/redo of them needn't
    /// re-read history.
    recent: RecentTxns,
    // Declared last so the lock is released after the connection closes.
    _lock: Option<LockFile>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Store {
    /// Creates a new workbook file with one sheet. Fails if the file exists.
    pub fn create(path: &Path) -> Result<Self, StoreError> {
        if path.exists() {
            return Err(StoreError::Exists(path.to_path_buf()));
        }
        let lock = LockFile::acquire(path)?;
        let conn = Connection::open(path)?;
        let mut store = Self::init(conn, Some(path.to_path_buf()), Some(lock))?;
        store.seed()?;
        Ok(store)
    }

    /// Opens an existing workbook for writing, taking the lock.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if !path.exists() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} not found", path.display()),
            )));
        }
        let lock = LockFile::acquire(path)?;
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut store = Self::init(conn, Some(path.to_path_buf()), Some(lock))?;
        if store.sheet_count()? == 0 {
            store.seed()?;
        }
        Ok(store)
    }

    /// Opens a workbook read-only, without the lock. Mutations fail with
    /// [`StoreError::ReadOnly`].
    pub fn open_read_only(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(map_open_err)?;
        schema::check_readable(&conn).map_err(map_store_open_err)?;
        let mut store = Self {
            conn,
            path: Some(path.to_path_buf()),
            read_only: true,
            undo: Vec::new(),
            redo: Vec::new(),
            recent: RecentTxns::default(),
            _lock: None,
        };
        store.rebuild_stacks()?;
        Ok(store)
    }

    /// An in-memory workbook with one sheet (tests and scratch use).
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        let mut store = Self::init(conn, None, None)?;
        store.seed()?;
        Ok(store)
    }

    fn init(
        mut conn: Connection,
        path: Option<PathBuf>,
        lock: Option<LockFile>,
    ) -> Result<Self, StoreError> {
        schema::configure(&conn).map_err(map_store_open_err)?;
        schema::migrate(&mut conn).map_err(map_store_open_err)?;
        let mut store = Self {
            conn,
            path,
            read_only: false,
            undo: Vec::new(),
            redo: Vec::new(),
            recent: RecentTxns::default(),
            _lock: lock,
        };
        store.rebuild_stacks()?;
        Ok(store)
    }

    fn seed(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO workbook(key, value) VALUES ('created', ?1)",
            [now_ms().to_string()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO workbook(key, value) VALUES ('app_version', ?1)",
            [env!("CARGO_PKG_VERSION")],
        )?;
        let id = next_id(&tx, KEY_NEXT_SHEET, "SELECT max(id) FROM sheets")?;
        tx.execute(
            "INSERT INTO sheets(id, name, position) VALUES (?1, ?2, 0)",
            params![id, DEFAULT_SHEET],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn sheet_count(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM sheets", [], |r| r.get(0))?)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Loads the materialised current state.
    pub fn load(&self) -> Result<Workbook, StoreError> {
        let mut wb = Workbook::default();
        {
            let mut st = self.conn.prepare("SELECT id, name, position FROM sheets")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let id = SheetId(r.get::<_, u32>(0)?);
                wb.sheets
                    .insert(id, Sheet::new(id, r.get::<_, String>(1)?, r.get(2)?));
            }
        }
        {
            let mut st = self
                .conn
                .prepare("SELECT sheet_id, row, col, vtype, value, meta FROM cells")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let a = codec::addr(r.get(0)?, r.get(1)?, r.get(2)?)?;
                let cell = decode_cell(r.get(3)?, r.get_ref(4)?, r.get(5)?)?;
                if let Some(sheet) = wb.sheets.get_mut(&a.sheet) {
                    sheet.cells.set(a.row, a.col, cell);
                }
            }
        }
        {
            let mut st = self
                .conn
                .prepare("SELECT sheet_id, axis, idx, size FROM layout")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let sheet = SheetId(r.get(0)?);
                let axis =
                    Axis::from_int(r.get(1)?).ok_or(StoreError::Corrupt("bad layout axis"))?;
                if let Some(s) = wb.sheets.get_mut(&sheet) {
                    s.layout.axis_mut(axis).insert(r.get(2)?, r.get(3)?);
                }
            }
        }
        {
            let mut st = self.conn.prepare("SELECT id, spec FROM charts")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let rec: ChartRecord = serde_json::from_str(&r.get::<_, String>(1)?)
                    .map_err(|_| StoreError::Corrupt("bad chart record"))?;
                wb.charts.insert(ChartId(r.get(0)?), rec);
            }
        }
        {
            let mut st = self
                .conn
                .prepare("SELECT hash, canonical FROM metatables")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let hash = codec::decode_meta(Some(r.get(0)?))?
                    .ok_or(StoreError::Corrupt("bad meta hash"))?;
                let src = MetaSource::from_canonical(&r.get::<_, String>(1)?)?;
                wb.metas.insert(hash, Arc::new(src));
            }
        }
        wb.settings = self.settings()?.into_iter().collect();
        Ok(wb)
    }

    /// User settings stored in the workbook (not part of undo history).
    pub fn settings(&self) -> Result<Vec<(String, String)>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT key, value FROM workbook WHERE key LIKE 'settings.%' ORDER BY key")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (k, v) = row?;
            out.push((k[SETTINGS_PREFIX.len()..].to_string(), v));
        }
        Ok(out)
    }

    /// Writes a workbook setting outside the undo history (for example
    /// colour assignments).
    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        self.writable()?;
        self.conn.execute(
            "INSERT OR REPLACE INTO workbook(key, value) VALUES (?1, ?2)",
            params![format!("{SETTINGS_PREFIX}{key}"), value],
        )?;
        Ok(())
    }

    fn writable(&self) -> Result<(), StoreError> {
        if self.read_only {
            Err(StoreError::ReadOnly)
        } else {
            Ok(())
        }
    }

    /// Applies a transaction. Returns `None` if nothing changed (no history
    /// entry is written).
    pub fn apply(&mut self, input: TxnInput) -> Result<Option<Committed>, StoreError> {
        self.writable()?;
        if input.ops.len() > 1 && input.ops.iter().any(|o| matches!(o, Op::Structure(_))) {
            return Err(StoreError::StructureNotAlone);
        }
        let tx = self.conn.transaction()?;
        let txn_id = insert_txn(&tx, &input.label, TxnKind::Normal, None)?;
        for meta in &input.metas {
            tx.execute(
                "INSERT OR IGNORE INTO metatables(hash, canonical, created_txn) VALUES (?1, ?2, ?3)",
                params![meta.hash().0.to_vec(), meta.canonical(), txn_id],
            )?;
        }
        let mut plan = Plan::default();
        for op in input.ops {
            plan_op(&tx, op, &mut plan)?;
        }
        let changes = canonical_order(plan.finish());
        if changes.is_empty() {
            // Dropping the transaction rolls back the txn row (and any
            // metatables only it would have introduced).
            return Ok(None);
        }
        write_records(&tx, txn_id, &changes)?;
        let info = read_txn(&tx, txn_id)?;
        tx.commit()?;
        self.undo.push(txn_id);
        self.redo.clear();
        self.recent.insert(txn_id, &changes);
        Ok(Some(Committed { txn: info, changes }))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_stack(&self) -> &[TxnId] {
        &self.undo
    }

    pub fn redo_stack(&self) -> &[TxnId] {
        &self.redo
    }

    /// The label of the action that undo would revert.
    pub fn undo_label(&self) -> Result<Option<String>, StoreError> {
        self.undo
            .last()
            .map(|id| origin_label(&self.conn, *id))
            .transpose()
    }

    pub fn redo_label(&self) -> Result<Option<String>, StoreError> {
        self.redo
            .last()
            .map(|id| origin_label(&self.conn, *id))
            .transpose()
    }

    pub fn undo(&mut self) -> Result<Option<Committed>, StoreError> {
        self.writable()?;
        let Some(&target) = self.undo.last() else {
            return Ok(None);
        };
        let label = format!("Undo {}", origin_label(&self.conn, target)?);
        let committed = self.revert(target, TxnKind::Undo, &label)?;
        self.undo.pop();
        self.redo.push(committed.txn.id);
        Ok(Some(committed))
    }

    pub fn redo(&mut self) -> Result<Option<Committed>, StoreError> {
        self.writable()?;
        let Some(&target) = self.redo.last() else {
            return Ok(None);
        };
        let label = format!("Redo {}", origin_label(&self.conn, target)?);
        let committed = self.revert(target, TxnKind::Redo, &label)?;
        self.redo.pop();
        self.undo.push(committed.txn.id);
        Ok(Some(committed))
    }

    /// Writes a txn that inverts `target`.
    fn revert(
        &mut self,
        target: TxnId,
        kind: TxnKind,
        label: &str,
    ) -> Result<Committed, StoreError> {
        let tx = self.conn.transaction()?;
        let original = match self.recent.get(target) {
            Some(c) => c,
            None => std::sync::Arc::new(load_txn_changes(&tx, target)?),
        };
        let txn_id = insert_txn(&tx, label, kind, Some(target))?;
        let mut changes = Vec::with_capacity(original.len());
        for change in canonical_order(invert_changes(&original)) {
            let change = refresh_before(&tx, change)?;
            apply_db(&tx, &change)?;
            changes.push(change);
        }
        write_records(&tx, txn_id, &changes)?;
        let info = read_txn(&tx, txn_id)?;
        tx.commit()?;
        self.recent.insert(txn_id, &changes);
        Ok(Committed { txn: info, changes })
    }

    /// Sets a cell back to the after-image it had in `version`.
    ///
    /// `version_addr` is the cell's address as of that transaction (see
    /// [`CellVersion::addr`]); `addr` is where the cell is now.
    pub fn restore(
        &mut self,
        addr: CellAddr,
        version: TxnId,
        version_addr: CellAddr,
    ) -> Result<Option<Committed>, StoreError> {
        self.writable()?;
        let after = self
            .conn
            .query_row(
                "SELECT a_vtype, a_value, a_meta FROM cell_changes
                 WHERE sheet_id=?1 AND row=?2 AND col=?3 AND txn=?4",
                params![
                    version_addr.sheet.0,
                    version_addr.row,
                    version_addr.col,
                    version
                ],
                |r| Ok(decode_cell(r.get(0)?, r.get_ref(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or(StoreError::NoSuchVersion(version))??;
        let label = format!("Restore {addr}");
        let tx = self.conn.transaction()?;
        let txn_id = insert_txn(&tx, &label, TxnKind::Restore, Some(version))?;
        let mut plan = Plan::default();
        plan_op(&tx, Op::SetCells(vec![(addr, after)]), &mut plan)?;
        let changes = plan.finish();
        if changes.is_empty() {
            return Ok(None);
        }
        write_records(&tx, txn_id, &changes)?;
        let info = read_txn(&tx, txn_id)?;
        tx.commit()?;
        self.undo.push(txn_id);
        self.redo.clear();
        self.recent.insert(txn_id, &changes);
        Ok(Some(Committed { txn: info, changes }))
    }

    /// Rebuilds the undo and redo stacks by replaying the txn log.
    fn rebuild_stacks(&mut self) -> Result<(), StoreError> {
        self.undo.clear();
        self.redo.clear();
        let mut st = self
            .conn
            .prepare("SELECT id, kind, target FROM txns ORDER BY id")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let id: TxnId = r.get(0)?;
            let kind = TxnKind::from_int(r.get(1)?)?;
            let target: Option<TxnId> = r.get(2)?;
            match kind {
                TxnKind::Normal | TxnKind::Restore => {
                    self.undo.push(id);
                    self.redo.clear();
                }
                TxnKind::Undo => {
                    if self.undo.last() == target.as_ref() {
                        self.undo.pop();
                    }
                    self.redo.push(id);
                }
                TxnKind::Redo => {
                    if self.redo.last() == target.as_ref() {
                        self.redo.pop();
                    }
                    self.undo.push(id);
                }
            }
        }
        Ok(())
    }

    /// The changes a txn made, in application order.
    pub fn txn_changes(&self, id: TxnId) -> Result<Vec<Change>, StoreError> {
        load_txn_changes(&self.conn, id)
    }

    pub fn txn(&self, id: TxnId) -> Result<TxnInfo, StoreError> {
        read_txn(&self.conn, id)
    }

    /// All txns in id order.
    pub fn txns(&self) -> Result<Vec<TxnInfo>, StoreError> {
        let mut st = self
            .conn
            .prepare("SELECT id, ts, label, kind, target FROM txns ORDER BY id")?;
        let rows = st.query_map([], txn_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(txn_info(r?)?);
        }
        Ok(out)
    }

    /// Every version of a cell, newest first. Follows the cell back through
    /// structural operations, so a cell that moved keeps its history.
    pub fn cell_history(&self, addr: CellAddr) -> Result<Vec<CellVersion>, StoreError> {
        // Structural txns on this sheet, newest first.
        let mut structs: Vec<(TxnId, StructOp)> = Vec::new();
        {
            let mut st = self.conn.prepare(
                "SELECT txn, after FROM other_changes WHERE entity='structure' ORDER BY txn DESC",
            )?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let txn: TxnId = r.get(0)?;
                let after: Option<String> = r.get(1)?;
                let op: StructOp = after
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .ok_or(StoreError::Corrupt("bad structure record"))?;
                if op.sheet() == addr.sheet {
                    structs.push((txn, op));
                }
            }
        }
        let mut st = self.conn.prepare_cached(
            "SELECT c.txn, c.b_vtype, c.b_value, c.b_meta, c.a_vtype, c.a_value, c.a_meta,
                    t.ts, t.label, t.kind, t.target
             FROM cell_changes c JOIN txns t ON t.id = c.txn
             WHERE c.sheet_id=?1 AND c.row=?2 AND c.col=?3 AND c.txn < ?4 AND c.txn > ?5
             ORDER BY c.txn DESC",
        )?;
        let mut query = |a: CellAddr, upper: TxnId, lower: TxnId, out: &mut Vec<CellVersion>| {
            let mut rows = st.query(params![a.sheet.0, a.row, a.col, upper, lower])?;
            while let Some(r) = rows.next()? {
                out.push(CellVersion {
                    txn: txn_info((r.get(0)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?))?,
                    addr: a,
                    before: decode_cell(r.get(1)?, r.get_ref(2)?, r.get(3)?)?,
                    after: decode_cell(r.get(4)?, r.get_ref(5)?, r.get(6)?)?,
                });
            }
            Ok::<_, StoreError>(())
        };
        let mut out = Vec::new();
        let mut a = addr;
        let mut upper = TxnId::MAX;
        for (txn, op) in structs {
            // Changes between this structural txn and the previous boundary
            // use the current coordinates.
            query(a, upper, txn, &mut out)?;
            // Inside an insert-type structural txn, cell changes (restores)
            // use post-op coordinates.
            if !op.is_delete() {
                query(a, txn + 1, txn - 1, &mut out)?;
            }
            match op.inverse().map_addr(a) {
                Some(prev) => a = prev,
                None => return Ok(out),
            }
            upper = txn;
        }
        query(a, upper, 0, &mut out)?;
        Ok(out)
    }

    /// Every cell, on any sheet, whose metatable is `hash`.
    pub fn share_set(&self, hash: &MetaHash) -> Result<Vec<CellAddr>, StoreError> {
        let mut st = self.conn.prepare_cached(
            "SELECT sheet_id, row, col FROM cells WHERE meta = ?1 ORDER BY sheet_id, row, col",
        )?;
        let rows = st.query_map([hash.0.to_vec()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (s, row, col) = r?;
            out.push(codec::addr(s, row, col)?);
        }
        Ok(out)
    }

    pub fn metatable(&self, hash: &MetaHash) -> Result<Option<MetaSource>, StoreError> {
        let canonical: Option<String> = self
            .conn
            .query_row(
                "SELECT canonical FROM metatables WHERE hash = ?1",
                [hash.0.to_vec()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(canonical
            .map(|c| MetaSource::from_canonical(&c))
            .transpose()?)
    }

    /// The id the next `AddSheet` without an explicit id would get.
    pub fn next_sheet_id(&self) -> Result<SheetId, StoreError> {
        let stored: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM workbook WHERE key = ?1",
                [KEY_NEXT_SHEET],
                |r| r.get(0),
            )
            .optional()?;
        let max: Option<u32> = self
            .conn
            .query_row("SELECT max(id) FROM sheets", [], |r| r.get(0))?;
        Ok(SheetId(
            stored
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(1)
                .max(max.map_or(1, |m| m + 1)),
        ))
    }

    /// Copies committed WAL pages into the main database file. Call when
    /// idle; it never blocks readers and does nothing for in-memory stores.
    pub fn checkpoint(&mut self) -> Result<(), StoreError> {
        if self.path.is_some() && !self.read_only {
            self.conn
                .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))?;
        }
        Ok(())
    }

    /// Writes a compacted copy to `path` (`VACUUM INTO`) and switches this
    /// store to it.
    pub fn save_as(&mut self, path: &Path) -> Result<(), StoreError> {
        if path.exists() {
            return Err(StoreError::Exists(path.to_path_buf()));
        }
        let lock = LockFile::acquire(path)?;
        let target = path
            .to_str()
            .ok_or(StoreError::Corrupt("path is not valid UTF-8"))?;
        self.conn.execute("VACUUM INTO ?1", [target])?;
        let conn = Connection::open(path)?;
        let fresh = Self::init(conn, Some(path.to_path_buf()), Some(lock))?;
        *self = fresh;
        Ok(())
    }
}

fn map_open_err(e: rusqlite::Error) -> StoreError {
    if let rusqlite::Error::SqliteFailure(f, _) = &e
        && f.code == rusqlite::ErrorCode::NotADatabase
    {
        return StoreError::NotADecelFile;
    }
    StoreError::Sqlite(e)
}

fn map_store_open_err(e: StoreError) -> StoreError {
    match e {
        StoreError::Sqlite(e) => map_open_err(e),
        other => other,
    }
}

type TxnRow = (TxnId, i64, String, i64, Option<TxnId>);

fn txn_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TxnRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
}

fn txn_info((id, ts, label, kind, target): TxnRow) -> Result<TxnInfo, StoreError> {
    Ok(TxnInfo {
        id,
        ts,
        label,
        kind: TxnKind::from_int(kind)?,
        target,
    })
}

fn read_txn(conn: &Connection, id: TxnId) -> Result<TxnInfo, StoreError> {
    txn_info(conn.query_row(
        "SELECT id, ts, label, kind, target FROM txns WHERE id = ?1",
        [id],
        txn_row,
    )?)
}

/// Follows undo/redo targets back to the original action's label.
fn origin_label(conn: &Connection, mut id: TxnId) -> Result<String, StoreError> {
    // Targets always point to earlier txns; a file where they don't is
    // corrupt (and following it could loop forever).
    loop {
        let info = read_txn(conn, id)?;
        match (info.kind, info.target) {
            (TxnKind::Undo | TxnKind::Redo, Some(t)) if t < id => id = t,
            (TxnKind::Undo | TxnKind::Redo, Some(_)) => {
                return Err(StoreError::Corrupt("txn target is not earlier"));
            }
            _ => return Ok(info.label),
        }
    }
}

fn insert_txn(
    tx: &Transaction<'_>,
    label: &str,
    kind: TxnKind,
    target: Option<TxnId>,
) -> Result<TxnId, StoreError> {
    tx.execute(
        "INSERT INTO txns(ts, label, kind, target) VALUES (?1, ?2, ?3, ?4)",
        params![now_ms(), label, kind.to_int(), target],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Allocates the next id from a monotonic counter in the `workbook` table.
fn next_id(conn: &Connection, key: &str, max_sql: &str) -> Result<u32, StoreError> {
    let stored: Option<String> = conn
        .query_row("SELECT value FROM workbook WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    let max_existing: Option<u32> = conn.query_row(max_sql, [], |r| r.get(0))?;
    let next = stored
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1)
        .max(max_existing.map_or(1, |m| m + 1));
    conn.execute(
        "INSERT OR REPLACE INTO workbook(key, value) VALUES (?1, ?2)",
        params![key, (next + 1).to_string()],
    )?;
    Ok(next)
}

/// A bounded cache of recent txns' changes, keyed by txn id.
#[derive(Default)]
struct RecentTxns {
    entries: std::collections::VecDeque<(TxnId, Arc<Vec<Change>>)>,
    total: usize,
}

impl RecentTxns {
    /// Total changes kept across all cached txns.
    const CAPACITY: usize = 200_000;

    fn insert(&mut self, id: TxnId, changes: &[Change]) {
        if changes.len() > Self::CAPACITY {
            return;
        }
        self.total += changes.len();
        self.entries.push_back((id, Arc::new(changes.to_vec())));
        while self.total > Self::CAPACITY {
            match self.entries.pop_front() {
                Some((_, c)) => self.total -= c.len(),
                None => break,
            }
        }
    }

    fn get(&self, id: TxnId) -> Option<Arc<Vec<Change>>> {
        self.entries
            .iter()
            .rev()
            .find(|(i, _)| *i == id)
            .map(|(_, c)| c.clone())
    }
}

// ---------------------------------------------------------------------------
// Reading current state

fn read_cell(conn: &Connection, a: CellAddr) -> Result<Cell, StoreError> {
    let mut st = conn.prepare_cached(
        "SELECT vtype, value, meta FROM cells WHERE sheet_id=?1 AND row=?2 AND col=?3",
    )?;
    let mut rows = st.query(params![a.sheet.0, a.row, a.col])?;
    match rows.next()? {
        Some(r) => decode_cell(r.get(0)?, r.get_ref(1)?, r.get(2)?),
        None => Ok(Cell::EMPTY),
    }
}

fn read_sheets(conn: &Connection) -> Result<Vec<(SheetId, SheetRecord)>, StoreError> {
    let mut st =
        conn.prepare_cached("SELECT id, name, position FROM sheets ORDER BY position, id")?;
    let rows = st.query_map([], |r| {
        Ok((
            SheetId(r.get(0)?),
            SheetRecord {
                name: r.get(1)?,
                position: r.get(2)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn read_sheet(conn: &Connection, id: SheetId) -> Result<Option<SheetRecord>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT name, position FROM sheets WHERE id = ?1",
            [id.0],
            |r| {
                Ok(SheetRecord {
                    name: r.get(0)?,
                    position: r.get(1)?,
                })
            },
        )
        .optional()?)
}

fn read_layout(
    conn: &Connection,
    sheet: SheetId,
    axis: Axis,
    idx: u32,
) -> Result<Option<f64>, StoreError> {
    let mut st =
        conn.prepare_cached("SELECT size FROM layout WHERE sheet_id=?1 AND axis=?2 AND idx=?3")?;
    Ok(st
        .query_row(params![sheet.0, axis.to_int(), idx], |r| r.get(0))
        .optional()?)
}

fn read_chart(conn: &Connection, id: ChartId) -> Result<Option<ChartRecord>, StoreError> {
    let spec: Option<String> = conn
        .query_row("SELECT spec FROM charts WHERE id = ?1", [id.0], |r| {
            r.get(0)
        })
        .optional()?;
    spec.map(|s| serde_json::from_str(&s).map_err(|_| StoreError::Corrupt("bad chart record")))
        .transpose()
}

fn meta_exists(conn: &Connection, hash: &MetaHash) -> Result<bool, StoreError> {
    let mut st = conn.prepare_cached("SELECT 1 FROM metatables WHERE hash = ?1")?;
    Ok(st.exists([hash.0.to_vec()])?)
}

/// Replaces a change's before-image with the current state.
fn refresh_before(conn: &Connection, change: Change) -> Result<Change, StoreError> {
    Ok(match change {
        // Undo/redo only ever revert the top of the stack, so a cell's
        // current state is exactly the inverted change's before-image. Skipping
        // the re-read keeps undo of large fills fast.
        c @ Change::Cell { .. } => c,
        Change::Sheet { id, after, .. } => Change::Sheet {
            id,
            before: read_sheet(conn, id)?,
            after,
        },
        Change::Layout {
            sheet,
            axis,
            idx,
            after,
            ..
        } => Change::Layout {
            sheet,
            axis,
            idx,
            before: read_layout(conn, sheet, axis, idx)?,
            after,
        },
        Change::Chart { id, after, .. } => Change::Chart {
            id,
            before: read_chart(conn, id)?,
            after,
        },
        s @ Change::Structure(_) => s,
    })
}

// ---------------------------------------------------------------------------
// Writing current state

/// Applies a change's after-image to the current-state tables.
fn apply_db(conn: &Connection, change: &Change) -> Result<(), StoreError> {
    match change {
        Change::Cell { addr, after, .. } => {
            if after.is_empty() {
                conn.prepare_cached("DELETE FROM cells WHERE sheet_id=?1 AND row=?2 AND col=?3")?
                    .execute(params![addr.sheet.0, addr.row, addr.col])?;
            } else {
                let (vt, v) = encode_raw(&after.raw)?;
                conn.prepare_cached(
                    "INSERT INTO cells(sheet_id, row, col, vtype, value, meta)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(sheet_id, row, col)
                     DO UPDATE SET vtype=excluded.vtype, value=excluded.value, meta=excluded.meta",
                )?
                .execute(params![
                    addr.sheet.0,
                    addr.row,
                    addr.col,
                    vt,
                    v,
                    encode_meta(&after.meta)
                ])?;
            }
        }
        Change::Sheet { id, after, .. } => match after {
            Some(rec) => {
                conn.prepare_cached(
                    "INSERT INTO sheets(id, name, position) VALUES (?1, ?2, ?3)
                     ON CONFLICT(id) DO UPDATE SET name=excluded.name, position=excluded.position",
                )?
                .execute(params![id.0, rec.name, rec.position])?;
            }
            None => {
                conn.execute("DELETE FROM sheets WHERE id = ?1", [id.0])?;
            }
        },
        Change::Layout {
            sheet,
            axis,
            idx,
            after,
            ..
        } => match after {
            Some(size) => {
                conn.prepare_cached(
                    "INSERT INTO layout(sheet_id, axis, idx, size) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(sheet_id, axis, idx) DO UPDATE SET size=excluded.size",
                )?
                .execute(params![sheet.0, axis.to_int(), idx, size])?;
            }
            None => {
                conn.prepare_cached("DELETE FROM layout WHERE sheet_id=?1 AND axis=?2 AND idx=?3")?
                    .execute(params![sheet.0, axis.to_int(), idx])?;
            }
        },
        Change::Chart { id, after, .. } => match after {
            Some(rec) => {
                let spec =
                    serde_json::to_string(rec).map_err(|_| StoreError::Corrupt("chart encode"))?;
                conn.execute(
                    "INSERT INTO charts(id, sheet_id, spec) VALUES (?1, ?2, ?3)
                     ON CONFLICT(id) DO UPDATE SET sheet_id=excluded.sheet_id, spec=excluded.spec",
                    params![id.0, rec.sheet.0, spec],
                )?;
            }
            None => {
                conn.execute("DELETE FROM charts WHERE id = ?1", [id.0])?;
            }
        },
        Change::Structure(op) => apply_structure_db(conn, *op)?,
    }
    Ok(())
}

/// Shifts cells, layout and chart anchors for a structural op. Two passes
/// through negative indices avoid transient primary-key clashes.
fn apply_structure_db(conn: &Connection, op: StructOp) -> Result<(), StoreError> {
    let sheet = op.sheet().0;
    let (at, n) = op.span();
    let (at, n) = (i64::from(at), i64::from(n));
    let (col, axis) = match op.axis() {
        Axis::Row => ("row", Axis::Row),
        Axis::Col => ("col", Axis::Col),
    };
    let ax = axis.to_int();
    if op.is_delete() {
        conn.execute(
            &format!("DELETE FROM cells WHERE sheet_id=?1 AND {col} >= ?2 AND {col} < ?3"),
            params![sheet, at, at + n],
        )?;
        conn.execute(
            "DELETE FROM layout WHERE sheet_id=?1 AND axis=?2 AND idx >= ?3 AND idx < ?4",
            params![sheet, ax, at, at + n],
        )?;
        conn.execute(
            &format!(
                "UPDATE cells SET {col} = -1 - ({col} - ?3) WHERE sheet_id=?1 AND {col} >= ?2"
            ),
            params![sheet, at + n, n],
        )?;
        conn.execute(
            "UPDATE layout SET idx = -1 - (idx - ?4) WHERE sheet_id=?1 AND axis=?2 AND idx >= ?3",
            params![sheet, ax, at + n, n],
        )?;
    } else {
        conn.execute(
            &format!(
                "UPDATE cells SET {col} = -1 - ({col} + ?3) WHERE sheet_id=?1 AND {col} >= ?2"
            ),
            params![sheet, at, n],
        )?;
        conn.execute(
            "UPDATE layout SET idx = -1 - (idx + ?4) WHERE sheet_id=?1 AND axis=?2 AND idx >= ?3",
            params![sheet, ax, at, n],
        )?;
    }
    conn.execute(
        &format!("UPDATE cells SET {col} = -1 - {col} WHERE sheet_id=?1 AND {col} < 0"),
        params![sheet],
    )?;
    conn.execute(
        "UPDATE layout SET idx = -1 - idx WHERE sheet_id=?1 AND axis=?2 AND idx < 0",
        params![sheet, ax],
    )?;
    Ok(())
}

/// Writes the history records for a txn's changes.
fn write_records(conn: &Connection, txn: TxnId, changes: &[Change]) -> Result<(), StoreError> {
    let mut seq = 0i64;
    for change in changes {
        match change {
            Change::Cell {
                addr,
                before,
                after,
            } => {
                let (bvt, bv) = encode_raw(&before.raw)?;
                let (avt, av) = encode_raw(&after.raw)?;
                conn.prepare_cached(
                    "INSERT INTO cell_changes(txn, sheet_id, row, col,
                        b_vtype, b_value, b_meta, a_vtype, a_value, a_meta)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )?
                .execute(params![
                    txn,
                    addr.sheet.0,
                    addr.row,
                    addr.col,
                    bvt,
                    bv,
                    encode_meta(&before.meta),
                    avt,
                    av,
                    encode_meta(&after.meta)
                ])?;
            }
            other => {
                if let Some(rec) = encode_other(other)? {
                    conn.prepare_cached(
                        "INSERT INTO other_changes(txn, seq, entity, before, after)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                    )?
                    .execute(params![txn, seq, rec.entity, rec.before, rec.after])?;
                    seq += 1;
                }
            }
        }
    }
    Ok(())
}

fn load_txn_changes(conn: &Connection, txn: TxnId) -> Result<Vec<Change>, StoreError> {
    let mut cells = Vec::new();
    {
        let mut st = conn.prepare_cached(
            "SELECT sheet_id, row, col, b_vtype, b_value, b_meta, a_vtype, a_value, a_meta
             FROM cell_changes WHERE txn = ?1",
        )?;
        let mut rows = st.query([txn])?;
        while let Some(r) = rows.next()? {
            cells.push(Change::Cell {
                addr: codec::addr(r.get(0)?, r.get(1)?, r.get(2)?)?,
                before: decode_cell(r.get(3)?, r.get_ref(4)?, r.get(5)?)?,
                after: decode_cell(r.get(6)?, r.get_ref(7)?, r.get(8)?)?,
            });
        }
    }
    let mut others = Vec::new();
    {
        let mut st = conn.prepare_cached(
            "SELECT entity, before, after FROM other_changes WHERE txn = ?1 ORDER BY seq",
        )?;
        let mut rows = st.query([txn])?;
        while let Some(r) = rows.next()? {
            others.push(decode_other(&OtherRecord {
                entity: r.get(0)?,
                before: r.get(1)?,
                after: r.get(2)?,
            })?);
        }
    }
    let mut all = cells;
    all.extend(others);
    Ok(canonical_order(all))
}

/// Orders a txn's changes so that applying them in sequence is valid:
///
/// - with a delete-rows/cols op: cell clears, then the other changes in their
///   recorded order (span clears, then the shift);
/// - with an insert op: the other changes (shift first), then cells;
/// - otherwise: sheet creations, other non-cell changes, cells, then sheet
///   deletions.
fn canonical_order(changes: Vec<Change>) -> Vec<Change> {
    let structure = changes.iter().find_map(|c| match c {
        Change::Structure(op) => Some(*op),
        _ => None,
    });
    let (cells, others): (Vec<Change>, Vec<Change>) = changes
        .into_iter()
        .partition(|c| matches!(c, Change::Cell { .. }));
    match structure {
        Some(op) if op.is_delete() => cells.into_iter().chain(others).collect(),
        Some(_) => others.into_iter().chain(cells).collect(),
        None => {
            let is_create = |c: &Change| {
                matches!(
                    c,
                    Change::Sheet {
                        before: None,
                        after: Some(_),
                        ..
                    }
                )
            };
            let is_delete = |c: &Change| {
                matches!(
                    c,
                    Change::Sheet {
                        before: Some(_),
                        after: None,
                        ..
                    }
                )
            };
            let creates = others.iter().filter(|c| is_create(c)).cloned();
            let middle = others
                .iter()
                .filter(|c| !is_create(c) && !is_delete(c))
                .cloned();
            let deletes = others.iter().filter(|c| is_delete(c)).cloned();
            creates.chain(middle).chain(cells).chain(deletes).collect()
        }
    }
}

// ---------------------------------------------------------------------------
// Planning ops into changes

/// Changes accumulated for one txn. Repeated changes to the same cell are
/// merged so each cell appears once (the history primary key requires it).
#[derive(Default)]
struct Plan {
    changes: Vec<Change>,
    cell_index: HashMap<CellAddr, usize>,
    /// Sheets created in this txn: their cells are known to be empty.
    new_sheets: std::collections::HashSet<SheetId>,
}

impl Plan {
    fn push(&mut self, conn: &Connection, change: Change) -> Result<(), StoreError> {
        apply_db(conn, &change)?;
        if let Change::Cell { addr, after, .. } = &change
            && let Some(&i) = self.cell_index.get(addr)
        {
            if let Change::Cell { after: prev, .. } = &mut self.changes[i] {
                *prev = after.clone();
            }
            return Ok(());
        }
        if let Change::Cell { addr, .. } = &change {
            self.cell_index.insert(*addr, self.changes.len());
        }
        self.changes.push(change);
        Ok(())
    }

    fn finish(self) -> Vec<Change> {
        self.changes.into_iter().filter(|c| !c.is_noop()).collect()
    }
}

fn check_sheet_name(
    conn: &Connection,
    name: &str,
    except: Option<SheetId>,
) -> Result<(), StoreError> {
    if name.trim().is_empty() || name != name.trim() || name.chars().any(char::is_control) {
        return Err(StoreError::BadSheetName(name.to_string()));
    }
    let clash = read_sheets(conn)?
        .into_iter()
        .any(|(id, rec)| rec.name == name && Some(id) != except);
    if clash {
        return Err(StoreError::DuplicateSheetName(name.to_string()));
    }
    Ok(())
}

fn require_sheet(conn: &Connection, id: SheetId) -> Result<SheetRecord, StoreError> {
    read_sheet(conn, id)?.ok_or(StoreError::NoSuchSheet(id))
}

/// Emits position changes so that `order` holds positions 0..n.
fn reposition(
    conn: &Connection,
    order: &[(SheetId, SheetRecord)],
    plan: &mut Plan,
) -> Result<(), StoreError> {
    for (pos, (id, rec)) in order.iter().enumerate() {
        let pos = pos as u32;
        if rec.position != pos {
            plan.push(
                conn,
                Change::Sheet {
                    id: *id,
                    before: Some(rec.clone()),
                    after: Some(SheetRecord {
                        name: rec.name.clone(),
                        position: pos,
                    }),
                },
            )?;
        }
    }
    Ok(())
}

fn plan_op(tx: &Transaction<'_>, op: Op, plan: &mut Plan) -> Result<(), StoreError> {
    let conn: &Connection = tx;
    match op {
        Op::SetCells(cells) => {
            let mut known: HashMap<MetaHash, bool> = HashMap::new();
            let mut sheets: HashMap<SheetId, bool> = HashMap::new();
            for (addr, cell) in cells {
                let exists = match sheets.get(&addr.sheet) {
                    Some(e) => *e,
                    None => {
                        let e = read_sheet(conn, addr.sheet)?.is_some();
                        sheets.insert(addr.sheet, e);
                        e
                    }
                };
                if !exists {
                    return Err(StoreError::NoSuchSheet(addr.sheet));
                }
                if let Some(raw) = &cell.raw
                    && !raw.is_storable()
                {
                    return Err(StoreError::InvalidRaw);
                }
                if let Some(h) = cell.meta {
                    let ok = match known.get(&h) {
                        Some(k) => *k,
                        None => {
                            let k = meta_exists(conn, &h)?;
                            known.insert(h, k);
                            k
                        }
                    };
                    if !ok {
                        return Err(StoreError::UnknownMeta(h));
                    }
                }
                let before = if plan.new_sheets.contains(&addr.sheet) {
                    Cell::EMPTY
                } else {
                    read_cell(conn, addr)?
                };
                plan.push(
                    conn,
                    Change::Cell {
                        addr,
                        before,
                        after: cell,
                    },
                )?;
            }
        }
        Op::AddSheet { name, position, id } => {
            check_sheet_name(conn, &name, None)?;
            let mut order = read_sheets(conn)?;
            let next = next_id(conn, KEY_NEXT_SHEET, "SELECT max(id) FROM sheets")?;
            let id = match id {
                // Must be unused: never below the counter.
                Some(id) if id.0 >= next => {
                    conn.execute(
                        "INSERT OR REPLACE INTO workbook(key, value) VALUES (?1, ?2)",
                        params![KEY_NEXT_SHEET, (id.0 + 1).to_string()],
                    )?;
                    id
                }
                Some(id) => return Err(StoreError::SheetIdInUse(id)),
                None => SheetId(next),
            };
            let pos = position
                .unwrap_or(order.len() as u32)
                .min(order.len() as u32);
            let rec = SheetRecord {
                name,
                position: pos,
            };
            // Shift later sheets first, then create.
            order.insert(pos as usize, (id, rec.clone()));
            for (p, (sid, r)) in order.iter().enumerate() {
                if *sid != id && r.position != p as u32 {
                    plan.push(
                        conn,
                        Change::Sheet {
                            id: *sid,
                            before: Some(r.clone()),
                            after: Some(SheetRecord {
                                name: r.name.clone(),
                                position: p as u32,
                            }),
                        },
                    )?;
                }
            }
            plan.push(
                conn,
                Change::Sheet {
                    id,
                    before: None,
                    after: Some(rec),
                },
            )?;
            plan.new_sheets.insert(id);
        }
        Op::RenameSheet { id, name } => {
            let rec = require_sheet(conn, id)?;
            check_sheet_name(conn, &name, Some(id))?;
            plan.push(
                conn,
                Change::Sheet {
                    id,
                    before: Some(rec.clone()),
                    after: Some(SheetRecord {
                        name,
                        position: rec.position,
                    }),
                },
            )?;
        }
        Op::MoveSheet { id, position } => {
            require_sheet(conn, id)?;
            let mut order = read_sheets(conn)?;
            let from = order
                .iter()
                .position(|(i, _)| *i == id)
                .ok_or(StoreError::NoSuchSheet(id))?;
            let item = order.remove(from);
            let to = (position as usize).min(order.len());
            order.insert(to, item);
            reposition(conn, &order, plan)?;
        }
        Op::DeleteSheet { id } => {
            let rec = require_sheet(conn, id)?;
            let mut order = read_sheets(conn)?;
            if order.len() <= 1 {
                return Err(StoreError::LastSheet);
            }
            // Cells.
            let cells: Vec<(i64, i64)> = conn
                .prepare("SELECT row, col FROM cells WHERE sheet_id = ?1")?
                .query_map([id.0], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            for (row, col) in cells {
                let addr = codec::addr(i64::from(id.0), row, col)?;
                let before = read_cell(conn, addr)?;
                plan.push(
                    conn,
                    Change::Cell {
                        addr,
                        before,
                        after: Cell::EMPTY,
                    },
                )?;
            }
            // Layout.
            let layout: Vec<(i64, u32, f64)> = conn
                .prepare("SELECT axis, idx, size FROM layout WHERE sheet_id = ?1")?
                .query_map([id.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            for (axis, idx, size) in layout {
                let axis = Axis::from_int(axis).ok_or(StoreError::Corrupt("bad layout axis"))?;
                plan.push(
                    conn,
                    Change::Layout {
                        sheet: id,
                        axis,
                        idx,
                        before: Some(size),
                        after: None,
                    },
                )?;
            }
            // Charts.
            let charts: Vec<u32> = conn
                .prepare("SELECT id FROM charts WHERE sheet_id = ?1")?
                .query_map([id.0], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            for cid in charts {
                let cid = ChartId(cid);
                let before = read_chart(conn, cid)?;
                plan.push(
                    conn,
                    Change::Chart {
                        id: cid,
                        before,
                        after: None,
                    },
                )?;
            }
            plan.push(
                conn,
                Change::Sheet {
                    id,
                    before: Some(rec),
                    after: None,
                },
            )?;
            order.retain(|(i, _)| *i != id);
            reposition(conn, &order, plan)?;
        }
        Op::SetLayout {
            sheet,
            axis,
            idx,
            size,
        } => {
            require_sheet(conn, sheet)?;
            if let Some(s) = size
                && !(s.is_finite() && s > 0.0)
            {
                return Err(StoreError::InvalidSize);
            }
            let before = read_layout(conn, sheet, axis, idx)?;
            plan.push(
                conn,
                Change::Layout {
                    sheet,
                    axis,
                    idx,
                    before,
                    after: size,
                },
            )?;
        }
        Op::ChartUpsert { id, chart } => {
            require_sheet(conn, chart.sheet)?;
            let id = match id {
                Some(id) => id,
                None => ChartId(next_id(conn, KEY_NEXT_CHART, "SELECT max(id) FROM charts")?),
            };
            let before = read_chart(conn, id)?;
            plan.push(
                conn,
                Change::Chart {
                    id,
                    before,
                    after: Some(chart),
                },
            )?;
        }
        Op::ChartDelete { id } => {
            let before = read_chart(conn, id)?.ok_or(StoreError::NoSuchChart(id))?;
            plan.push(
                conn,
                Change::Chart {
                    id,
                    before: Some(before),
                    after: None,
                },
            )?;
        }
        Op::Structure(op) => plan_structure(conn, op, plan)?,
    }
    Ok(())
}

/// Plans a structural op. Deletions first record clears of everything in the
/// deleted span (cells, layout) and move chart anchors out of it, so the
/// shift itself is exactly invertible.
fn plan_structure(conn: &Connection, op: StructOp, plan: &mut Plan) -> Result<(), StoreError> {
    let sheet = op.sheet();
    require_sheet(conn, sheet)?;
    let (at, n) = op.span();
    if n == 0 {
        return Ok(());
    }
    if op.is_delete() {
        let end = at.saturating_add(n);
        let col = match op.axis() {
            Axis::Row => "row",
            Axis::Col => "col",
        };
        let cells: Vec<(i64, i64)> = conn
            .prepare(&format!(
                "SELECT row, col FROM cells WHERE sheet_id=?1 AND {col} >= ?2 AND {col} < ?3"
            ))?
            .query_map(params![sheet.0, at, end], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (row, c) in cells {
            let addr = codec::addr(i64::from(sheet.0), row, c)?;
            let before = read_cell(conn, addr)?;
            plan.push(
                conn,
                Change::Cell {
                    addr,
                    before,
                    after: Cell::EMPTY,
                },
            )?;
        }
        let layout: Vec<(u32, f64)> = conn
            .prepare("SELECT idx, size FROM layout WHERE sheet_id=?1 AND axis=?2 AND idx >= ?3 AND idx < ?4")?
            .query_map(params![sheet.0, op.axis().to_int(), at, end], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
        for (idx, size) in layout {
            plan.push(
                conn,
                Change::Layout {
                    sheet,
                    axis: op.axis(),
                    idx,
                    before: Some(size),
                    after: None,
                },
            )?;
        }
    }
    // Charts anywhere may anchor on or read from this sheet; record each
    // move explicitly so undo restores it exactly.
    let charts: Vec<(u32, String)> = conn
        .prepare("SELECT id, spec FROM charts")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (id, spec) in charts {
        let rec: ChartRecord =
            serde_json::from_str(&spec).map_err(|_| StoreError::Corrupt("bad chart record"))?;
        let moved = rec.shifted(op);
        if moved != rec {
            plan.push(
                conn,
                Change::Chart {
                    id: ChartId(id),
                    before: Some(rec),
                    after: Some(moved),
                },
            )?;
        }
    }
    plan.push(conn, Change::Structure(op))?;
    Ok(())
}
