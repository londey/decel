// SPDX-License-Identifier: MIT OR Apache-2.0

//! The command/event protocol between the UI and the engine thread (DESIGN §9.2).

use std::collections::BTreeMap;
use std::path::PathBuf;

use decel_model::{
    Axis, Cell, CellAddr, ChartId, ChartRecord, MetaHash, MetaSource, Rect, SheetId, StructOp,
    Value,
};
use decel_store::{CellVersion, TxnId, TxnInfo};

use crate::{CommandError, DisplayCell, Preview};

/// How a metatable edit applies (DESIGN §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaEditMode {
    /// Only the target cells are repointed to the new metatable.
    Cow,
    /// Every cell sharing the first target's metatable is repointed.
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillKind {
    /// Fill drag: repeat `src` over `dst`.
    Drag,
    /// Ctrl+D: the top row of `dst` over the rest.
    Down,
    /// Ctrl+R: the left column of `dst` over the rest.
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteMode {
    All,
    Values,
    Meta,
}

/// Clipboard contents: a rectangle of cells, row-major, with the metatable
/// sources they reference (so a paste works across workbooks).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClipData {
    pub rows: u32,
    pub cols: u32,
    pub cells: Vec<Cell>,
    pub metas: Vec<MetaSource>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Opens a workbook file. `read_only` skips the lock.
    Open {
        path: PathBuf,
        read_only: bool,
    },
    /// Creates a workbook. With no path it lives in memory until Save As.
    New {
        path: Option<PathBuf>,
    },
    SaveAs {
        path: PathBuf,
    },
    /// Parses and validates text, then commits it (or replies `Rejected`).
    EnterText {
        addr: CellAddr,
        text: String,
    },
    SetCells {
        label: String,
        cells: Vec<(CellAddr, Cell)>,
    },
    /// Sets (`Some`) or removes (`None`) the metatable on `targets`.
    SetMeta {
        targets: Vec<CellAddr>,
        source: Option<MetaSource>,
        mode: MetaEditMode,
    },
    /// Forks `targets` onto a private copy of their metatable.
    Detach {
        targets: Vec<CellAddr>,
    },
    Fill {
        kind: FillKind,
        sheet: SheetId,
        src: Rect,
        dst: Rect,
        with_values: bool,
        preview: bool,
    },
    AutoFill {
        addr: CellAddr,
        axis: Axis,
        preview: bool,
    },
    Paste {
        at: CellAddr,
        clip: ClipData,
        mode: PasteMode,
    },
    /// Builds clipboard contents for a rectangle.
    Copy {
        sheet: SheetId,
        rect: Rect,
    },
    Undo,
    Redo,
    Restore {
        addr: CellAddr,
        version: TxnId,
        version_addr: CellAddr,
    },
    /// Subscribes `id` to display updates for a rectangle (plus a margin).
    Subscribe {
        id: u32,
        sheet: SheetId,
        rect: Rect,
    },
    Unsubscribe {
        id: u32,
    },
    Preview {
        addr: CellAddr,
        source: MetaSource,
    },
    /// Text and hint for starting an edit (F2).
    BeginEdit {
        addr: CellAddr,
    },
    Tooltip {
        addr: CellAddr,
    },
    /// Ctrl+arrow target from `from` in direction `(dr, dc)`.
    DataEdge {
        from: CellAddr,
        dr: i64,
        dc: i64,
    },
    /// Final values of a rectangle (charts, CSV export, tests).
    Values {
        sheet: SheetId,
        rect: Rect,
    },
    QueryHistory {
        addr: CellAddr,
    },
    QueryShareSet {
        hash: MetaHash,
    },
    QueryMeta {
        hash: MetaHash,
    },
    /// Recent transactions for the undo-history view.
    QueryTxns {
        limit: usize,
    },
    AddSheet {
        name: String,
    },
    RenameSheet {
        id: SheetId,
        name: String,
    },
    MoveSheet {
        id: SheetId,
        position: u32,
    },
    DeleteSheet {
        id: SheetId,
    },
    SetLayout {
        sheet: SheetId,
        axis: Axis,
        idx: u32,
        size: Option<f64>,
    },
    Structure(StructOp),
    ChartUpsert {
        id: Option<ChartId>,
        chart: ChartRecord,
    },
    ChartDelete {
        id: ChartId,
    },
    /// Writes a workbook setting outside the undo history.
    SetSetting {
        key: String,
        value: String,
    },
    Shutdown,
}

/// A successful reply to a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Done,
    /// A transaction was committed, touching these cells.
    Committed {
        affected: Vec<CellAddr>,
    },
    /// Nothing needed changing.
    NoChange,
    /// The edit was refused (parse/validate failure).
    Rejected(String),
    Edit {
        text: String,
        hint: Option<String>,
    },
    Tooltip(Option<String>),
    Preview(Box<Preview>),
    /// Cells a fill/auto-fill would touch (preview variants).
    Addresses(Vec<CellAddr>),
    Values(Vec<(CellAddr, Value)>),
    Clip(ClipData),
    History(Vec<CellVersion>),
    ShareSet(Vec<CellAddr>),
    Meta(Option<MetaSource>),
    Txns {
        txns: Vec<TxnInfo>,
        /// Ids currently on the undo stack (bottom first).
        undo: Vec<TxnId>,
        redo: Vec<TxnId>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SheetInfo {
    pub id: SheetId,
    pub name: String,
    pub col_widths: BTreeMap<u32, f64>,
    pub row_heights: BTreeMap<u32, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkbookInfo {
    pub path: Option<PathBuf>,
    pub read_only: bool,
    /// In tab order.
    pub sheets: Vec<SheetInfo>,
    pub charts: BTreeMap<ChartId, ChartRecord>,
    pub settings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Ack {
        seq: u64,
        result: Result<Reply, CommandError>,
    },
    /// Display updates. With `full`, every other cell of `rect` is empty.
    CellsUpdated {
        sub: u32,
        sheet: SheetId,
        rect: Option<Rect>,
        full: bool,
        cells: Vec<(CellAddr, DisplayCell)>,
    },
    TxnCommitted {
        label: String,
        can_undo: bool,
        can_redo: bool,
        undo_label: Option<String>,
        redo_label: Option<String>,
    },
    RecalcProgress {
        pending: usize,
    },
    /// Workbook structure (sheets, layout, charts, settings) changed or a
    /// workbook was opened.
    Workbook(Box<WorkbookInfo>),
    Error {
        message: String,
    },
}
