// SPDX-License-Identifier: MIT OR Apache-2.0

//! The main window view: title bar, edit bar, grid, sheet tabs and status
//! bar (DESIGN §8.1). All engine access goes through commands; replies and
//! events arrive asynchronously.

use std::cell::Cell as StdCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use decel_engine::protocol::{
    ClipData, Command, Event, FillKind, MetaEditMode, MetaSummary, PasteMode, Reply, WorkbookInfo,
};
use decel_engine::{CommandError, DisplayCell, EngineHandle};
use decel_model::{Axis, Cell, CellAddr, MetaHash, Rect, SheetId};
use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    App, AppContext as _, Bounds, ClickEvent, ClipboardItem, Context, CursorStyle, Entity,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyDownEvent, KeyUpEvent,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, PathPromptOptions, Pixels, Point, Render, ScrollEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, point,
    prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputEvent, InputState, Position};
use gpui_component::{ActiveTheme as _, TitleBar};

use crate::actions::*;
use crate::dock::Dock;
use crate::grid::GridElement;
use crate::recent;
use crate::sizes::AxisSizes;

pub const HEADER_W: f64 = 56.0;
pub const HEADER_H: f64 = 24.0;
pub const DEFAULT_COL_W: f64 = 96.0;
pub const DEFAULT_ROW_H: f64 = 24.0;
const RESIZE_GRAB: f64 = 4.0;
const TOOLTIP_DELAY: Duration = Duration::from_millis(400);
/// Matches the engine's subscription margin.
const SUB_MARGIN: u32 = decel_engine::actor::MARGIN;
const GRID_SUB: u32 = 1;

type ReplyFn = Box<
    dyn FnOnce(&mut Workspace, Result<Reply, CommandError>, &mut Window, &mut Context<Workspace>),
>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelMode {
    Cells,
    Rows,
    Cols,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (u32, u32),
    pub cursor: (u32, u32),
    pub mode: SelMode,
}

impl Selection {
    fn at(row: u32, col: u32) -> Self {
        Self {
            anchor: (row, col),
            cursor: (row, col),
            mode: SelMode::Cells,
        }
    }

    pub fn rect(&self) -> Rect {
        let r = Rect::new(self.anchor.0, self.anchor.1, self.cursor.0, self.cursor.1);
        match self.mode {
            SelMode::Cells => r,
            SelMode::Rows => Rect::new(r.r1, 0, r.r2, u32::MAX),
            SelMode::Cols => Rect::new(0, r.c1, u32::MAX, r.c2),
        }
    }
}

pub(crate) struct EditState {
    addr: CellAddr,
    input: Entity<InputState>,
    _sub: Subscription,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Resize {
    Col { idx: u32, start: f64, origin: f64 },
    Row { idx: u32, start: f64, origin: f64 },
}

pub(crate) enum Banner {
    Locked { path: PathBuf, who: String },
    ReadOnly,
    Error(String),
}

#[derive(Default)]
pub struct Status {
    pub pending: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub last: String,
}

/// Frame-time and keypress-to-paint measurement (`DECEL_DEBUG_FPS=1`).
#[derive(Default)]
pub struct FpsMeter {
    frames: std::collections::VecDeque<Instant>,
    pub key_at: Option<Instant>,
    pub key_latency: Option<Duration>,
}

impl FpsMeter {
    pub fn frame(&mut self) {
        let now = Instant::now();
        self.frames.push_back(now);
        while self.frames.len() > 120 {
            self.frames.pop_front();
        }
        if let Some(k) = self.key_at.take() {
            self.key_latency = Some(now - k);
        }
    }

    pub fn fps(&self) -> f64 {
        match (self.frames.front(), self.frames.back()) {
            (Some(a), Some(b)) if self.frames.len() > 1 && b > a => {
                (self.frames.len() - 1) as f64 / (*b - *a).as_secs_f64()
            }
            _ => 0.0,
        }
    }
}

/// The share set under the pointer while Shift is held in the code side.
pub struct ShareHighlight {
    pub hash: MetaHash,
    pub cells: std::collections::HashSet<CellAddr>,
    pub total: usize,
    pub other_sheets: usize,
    pub at: Point<Pixels>,
}

/// An auto-fill waiting for confirmation.
pub struct AutoFillPreview {
    pub addr: CellAddr,
    pub axis: Axis,
    pub cells: Vec<CellAddr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockTab {
    Metatable,
    History,
    Chart,
}

/// Length of the code-side flip animation.
pub const FLIP: Duration = Duration::from_millis(150);

/// The key held for peek mode.
pub const PEEK_KEY: &str = "`";

pub enum Hit {
    Corner,
    ColHeader { col: u32, edge: Option<u32> },
    RowHeader { row: u32, edge: Option<u32> },
    Cell { row: u32, col: u32 },
    Outside,
}

pub struct Workspace {
    pub(crate) engine: EngineHandle,
    pub(crate) replies: HashMap<u64, ReplyFn>,
    pub(crate) focus: FocusHandle,
    pub(crate) info: Option<WorkbookInfo>,
    pub(crate) sheet: Option<SheetId>,
    pub(crate) cols: AxisSizes,
    pub(crate) rows: AxisSizes,
    pub(crate) cache: HashMap<CellAddr, DisplayCell>,
    pub(crate) subscribed: Option<(SheetId, Rect)>,
    /// Scroll offset of the body in content pixels (x, y).
    pub(crate) scroll: (f64, f64),
    pub(crate) sel: Selection,
    pub(crate) edit: Option<EditState>,
    /// Edit text of the selected cell (shown in the edit bar).
    pub(crate) bar_text: String,
    pub(crate) hint: Option<String>,
    pub(crate) edit_error: Option<String>,
    pub(crate) tooltip: Option<(CellAddr, String, Point<Pixels>)>,
    pub(crate) hover: Option<CellAddr>,
    pub(crate) _hover_task: Option<Task<()>>,
    pub(crate) resize: Option<Resize>,
    pub(crate) resize_hover: bool,
    pub(crate) dragging: bool,
    pub(crate) grid_bounds: Rc<StdCell<Bounds<Pixels>>>,
    pub(crate) status: Status,
    pub(crate) banner: Option<Banner>,
    pub(crate) show_recent: bool,
    pub(crate) fps: Option<FpsMeter>,
    pub(crate) _event_task: Task<()>,
    pub(crate) _keys: Subscription,
    pub(crate) dock: Dock,
    pub(crate) renaming: Option<crate::sheets::Renaming>,
    pub(crate) confirm_delete: Option<SheetId>,
    /// Switch to the sheet with this name when it appears (after Add sheet).
    pub(crate) switch_to_name: Option<String>,
    pub(crate) dock_tab: DockTab,
    pub(crate) charts: crate::charts::ChartsState,
    pub(crate) csv_import: Option<crate::csv_ui::ImportDialog>,
    pub(crate) csv_export: Option<crate::csv_ui::ExportDialog>,
    pub(crate) history: crate::history::HistoryState,
    pub(crate) dock_sync_pending: bool,
    pub(crate) peek: bool,
    pub(crate) code_side: bool,
    /// Start of the code-side flip animation.
    pub(crate) flip: Option<Instant>,
    pub(crate) boundaries: bool,
    pub(crate) legend: Option<Vec<MetaSummary>>,
    pub(crate) shift: bool,
    pub(crate) share_hl: Option<ShareHighlight>,
    /// Target rectangle while dragging the fill handle.
    pub(crate) fill_drag: Option<Rect>,
    pub(crate) autofill: Option<AutoFillPreview>,
    pub(crate) clip: Option<ClipData>,
    pub(crate) _activation: Option<Subscription>,
}

fn trace(msg: impl FnOnce() -> String) {
    if std::env::var("DECEL_DEBUG_KEYS").is_ok() {
        eprintln!("{}", msg());
    }
}

/// Extends a selection to include `(row, col)` along the dominant axis (the
/// fill handle drags in one direction at a time).
fn fill_target(sel: Rect, row: u32, col: u32) -> Rect {
    let dr = if row > sel.r2 {
        row - sel.r2
    } else {
        sel.r1.saturating_sub(row)
    };
    let dc = if col > sel.c2 {
        col - sel.c2
    } else {
        sel.c1.saturating_sub(col)
    };
    if dr >= dc {
        Rect::new(sel.r1.min(row), sel.c1, sel.r2.max(row), sel.c2)
    } else {
        Rect::new(sel.r1, sel.c1.min(col), sel.r2, sel.c2.max(col))
    }
}

fn addr_label(row: u32, col: u32) -> String {
    format!("{},{}", u64::from(row) + 1, u64::from(col) + 1)
}

impl Workspace {
    pub fn new(
        engine: EngineHandle,
        events: UnboundedReceiver<Event>,
        open: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let event_task = cx.spawn_in(window, async move |this, cx| {
            let mut events = events;
            while let Some(e) = events.next().await {
                if this
                    .update_in(cx, |this, window, cx| this.on_event(e, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        // Enter/Tab/Escape while editing are handled before the input's own
        // key bindings see them.
        let weak = cx.entity().downgrade();
        let keys = cx.intercept_keystrokes(move |ev, window, cx| {
            let Some(ws) = weak.upgrade() else { return };
            let (editing, dock_focused, renaming) = {
                let w = ws.read(cx);
                (
                    w.edit.is_some(),
                    w.dock.open && w.dock.editor.read(cx).focus_handle(cx).is_focused(window),
                    w.renaming.is_some(),
                )
            };
            let handled = if renaming {
                ws.update(cx, |ws, cx| ws.on_rename_key(&ev.keystroke, window, cx))
            } else if editing {
                ws.update(cx, |ws, cx| ws.on_editor_key(&ev.keystroke, window, cx))
            } else if dock_focused {
                ws.update(cx, |ws, cx| ws.on_dock_key(&ev.keystroke, window, cx))
            } else {
                false
            };
            if handled {
                cx.stop_propagation();
            }
        });
        let fps = std::env::var("DECEL_DEBUG_FPS")
            .is_ok_and(|v| v == "1")
            .then(FpsMeter::default);
        let mut this = Self {
            engine,
            replies: HashMap::new(),
            focus: cx.focus_handle(),
            info: None,
            sheet: None,
            cols: AxisSizes::new(DEFAULT_COL_W, &Default::default()),
            rows: AxisSizes::new(DEFAULT_ROW_H, &Default::default()),
            cache: HashMap::new(),
            subscribed: None,
            scroll: (0.0, 0.0),
            sel: Selection::at(0, 0),
            edit: None,
            bar_text: String::new(),
            hint: None,
            edit_error: None,
            tooltip: None,
            hover: None,
            _hover_task: None,
            resize: None,
            resize_hover: false,
            dragging: false,
            grid_bounds: Rc::new(StdCell::new(Bounds::default())),
            status: Status::default(),
            banner: None,
            show_recent: false,
            fps,
            _event_task: event_task,
            _keys: keys,
            dock: Dock::new(window, cx),
            renaming: None,
            confirm_delete: None,
            switch_to_name: None,
            dock_tab: DockTab::Metatable,
            charts: Default::default(),
            csv_import: None,
            csv_export: None,
            history: Default::default(),
            dock_sync_pending: false,
            peek: false,
            code_side: false,
            flip: None,
            boundaries: true,
            legend: None,
            shift: false,
            share_hl: None,
            fill_drag: None,
            autofill: None,
            clip: None,
            _activation: None,
        };
        this.init_dock(window, cx);
        // Losing window focus releases peek and the share modifier.
        this._activation = Some(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.peek = false;
                this.shift = false;
                this.share_hl = None;
                cx.notify();
            }
        }));
        let is_csv = open.as_ref().is_some_and(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "csv" | "tsv" | "txt"))
        });
        match open {
            // A CSV on the command line imports into a new workbook.
            Some(p) if is_csv && p.exists() => {
                this.new_file(None, window, cx);
                this.open_import_dialog(p, cx);
            }
            Some(p) if p.exists() => this.open_path(p, false, window, cx),
            Some(p) => this.new_file(Some(p), window, cx),
            None => this.new_file(None, window, cx),
        }
        this.focus.focus(window, cx);
        this
    }

    // -----------------------------------------------------------------------
    // Engine plumbing

    pub(crate) fn send(&mut self, cmd: Command) -> u64 {
        self.engine.send(cmd)
    }

    pub(crate) fn request(
        &mut self,
        cmd: Command,
        f: impl FnOnce(
            &mut Workspace,
            Result<Reply, CommandError>,
            &mut Window,
            &mut Context<Workspace>,
        ) + 'static,
    ) {
        let seq = self.send(cmd);
        self.replies.insert(seq, Box::new(f));
    }

    fn on_event(&mut self, e: Event, window: &mut Window, cx: &mut Context<Self>) {
        match e {
            Event::Ack { seq, result } => {
                if let Some(f) = self.replies.remove(&seq) {
                    f(self, result, window, cx);
                } else if let Err(err) = result {
                    self.banner = Some(Banner::Error(err.to_string()));
                }
            }
            Event::CellsUpdated {
                sub,
                sheet,
                full,
                cells,
                rect,
            } => {
                if self.on_chart_cells(sub, rect, full, &cells) {
                    cx.notify();
                    return;
                }
                if sub != GRID_SUB || Some(sheet) != self.sheet {
                    return;
                }
                // A full snapshot replaces everything we had for the sheet.
                if full {
                    self.cache.retain(|a, _| a.sheet != sheet);
                }
                for (a, d) in cells {
                    self.cache.insert(a, d);
                }
            }
            Event::TxnCommitted {
                label,
                can_undo,
                can_redo,
                undo_label,
                redo_label,
            } => {
                self.status.can_undo = can_undo;
                self.status.can_redo = can_redo;
                self.status.undo_label = undo_label;
                self.status.redo_label = redo_label;
                self.refresh_history();
                if !label.is_empty() {
                    self.status.last = label;
                }
                self.refresh_bar();
            }
            Event::RecalcProgress { pending } => self.status.pending = pending,
            Event::Workbook(info) => self.on_workbook(*info, window, cx),
            Event::Error { message } => self.banner = Some(Banner::Error(message)),
        }
        cx.notify();
    }

    fn on_workbook(&mut self, info: WorkbookInfo, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self
            .switch_to_name
            .as_ref()
            .and_then(|n| info.sheets.iter().find(|s| &s.name == n))
            .map(|s| s.id);
        if wanted.is_some() {
            self.switch_to_name = None;
        }
        let sheet = match wanted.or(self.sheet) {
            Some(s) if info.sheets.iter().any(|i| i.id == s) => s,
            _ => match info.sheets.first() {
                Some(s) => s.id,
                None => return,
            },
        };
        if let Some(p) = &info.path {
            recent::touch(p);
        }
        if info.read_only && !matches!(self.banner, Some(Banner::ReadOnly)) {
            self.banner = Some(Banner::ReadOnly);
        }
        self.info = Some(info);
        self.sync_chart_subs();
        if let Some(id) = self.charts.selected
            && !self
                .info
                .as_ref()
                .is_some_and(|i| i.charts.contains_key(&id))
        {
            self.select_chart(None, window, cx);
        }
        self.switch_sheet(sheet, window, cx);
    }

    fn sheet_info(&self) -> Option<&decel_engine::protocol::SheetInfo> {
        let id = self.sheet?;
        self.info.as_ref()?.sheets.iter().find(|s| s.id == id)
    }

    pub fn switch_sheet(&mut self, id: SheetId, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.sheet != Some(id);
        self.sheet = Some(id);
        if let Some((cols, rows)) = self.sheet_info().map(|s| {
            (
                AxisSizes::new(DEFAULT_COL_W, &s.col_widths),
                AxisSizes::new(DEFAULT_ROW_H, &s.row_heights),
            )
        }) {
            self.cols = cols;
            self.rows = rows;
        }
        if changed {
            self.cache.clear();
            self.scroll = (0.0, 0.0);
            self.sel = Selection::at(0, 0);
        }
        self.subscribed = None;
        self.update_title(window);
        self.ensure_subscribed();
        self.refresh_bar();
        cx.notify();
    }

    fn update_title(&self, window: &mut Window) {
        let Some(info) = &self.info else { return };
        let name = match &info.path {
            Some(p) if !recent::is_untitled(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            _ => "Untitled".into(),
        };
        let sheet = self
            .sheet_info()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let ro = if info.read_only { " [read-only]" } else { "" };
        window.set_window_title(&format!("{name} — {sheet}{ro} — decel"));
    }

    pub fn title(&self) -> String {
        let Some(info) = &self.info else {
            return "decel".into();
        };
        let name = match &info.path {
            Some(p) if !recent::is_untitled(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            _ => "Untitled".into(),
        };
        let sheet = self
            .sheet_info()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let ro = if info.read_only { " [read-only]" } else { "" };
        format!("{name} — {sheet}{ro}")
    }

    // -----------------------------------------------------------------------
    // Geometry

    /// Body size in pixels (excluding headers).
    fn body_size(&self) -> (f64, f64) {
        let b = self.grid_bounds.get();
        (
            (f64::from(b.size.width) - HEADER_W).max(0.0),
            (f64::from(b.size.height) - HEADER_H).max(0.0),
        )
    }

    /// The visible rectangle of cells.
    pub fn visible(&self) -> Rect {
        let (w, h) = self.body_size();
        let c1 = self.cols.index_at(self.scroll.0);
        let c2 = self.cols.index_at(self.scroll.0 + w.max(1.0));
        let r1 = self.rows.index_at(self.scroll.1);
        let r2 = self.rows.index_at(self.scroll.1 + h.max(1.0));
        Rect::new(r1, c1, r2, c2)
    }

    fn ensure_subscribed(&mut self) {
        let Some(sheet) = self.sheet else { return };
        let vis = self.visible();
        let covered = self.subscribed.is_some_and(|(s, r)| {
            let area = r.expand(SUB_MARGIN);
            s == sheet && area.contains(vis.r1, vis.c1) && area.contains(vis.r2, vis.c2)
        });
        if !covered {
            self.subscribed = Some((sheet, vis));
            self.send(Command::Subscribe {
                id: GRID_SUB,
                sheet,
                rect: vis,
            });
        }
    }

    pub fn hit(&self, pos: Point<Pixels>) -> Hit {
        let b = self.grid_bounds.get();
        let x = f64::from(pos.x - b.origin.x);
        let y = f64::from(pos.y - b.origin.y);
        if x < 0.0 || y < 0.0 || x > f64::from(b.size.width) || y > f64::from(b.size.height) {
            return Hit::Outside;
        }
        let cx = x - HEADER_W + self.scroll.0;
        let cy = y - HEADER_H + self.scroll.1;
        let col = self.cols.index_at(cx);
        let row = self.rows.index_at(cy);
        match (x < HEADER_W, y < HEADER_H) {
            (true, true) => Hit::Corner,
            (false, true) => {
                let right = self.cols.offset(col) + self.cols.size(col);
                let left = self.cols.offset(col);
                let edge = if (right - cx).abs() <= RESIZE_GRAB {
                    Some(col)
                } else if (cx - left).abs() <= RESIZE_GRAB && col > 0 {
                    Some(col - 1)
                } else {
                    None
                };
                Hit::ColHeader { col, edge }
            }
            (true, false) => {
                let bottom = self.rows.offset(row) + self.rows.size(row);
                let top = self.rows.offset(row);
                let edge = if (bottom - cy).abs() <= RESIZE_GRAB {
                    Some(row)
                } else if (cy - top).abs() <= RESIZE_GRAB && row > 0 {
                    Some(row - 1)
                } else {
                    None
                };
                Hit::RowHeader { row, edge }
            }
            (false, false) => Hit::Cell { row, col },
        }
    }

    /// Screen bounds of a cell (may be off-screen).
    fn cell_bounds(&self, row: u32, col: u32) -> Bounds<Pixels> {
        let b = self.grid_bounds.get();
        let x = HEADER_W + self.cols.offset(col) - self.scroll.0;
        let y = HEADER_H + self.rows.offset(row) - self.scroll.1;
        Bounds::new(
            point(b.origin.x + px(x as f32), b.origin.y + px(y as f32)),
            gpui::size(
                px(self.cols.size(col) as f32),
                px(self.rows.size(row) as f32),
            ),
        )
    }

    fn ensure_visible(&mut self, row: u32, col: u32) {
        let (w, h) = self.body_size();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let (x0, x1) = (
            self.cols.offset(col),
            self.cols.offset(col) + self.cols.size(col),
        );
        let (y0, y1) = (
            self.rows.offset(row),
            self.rows.offset(row) + self.rows.size(row),
        );
        if x0 < self.scroll.0 {
            self.scroll.0 = x0;
        } else if x1 > self.scroll.0 + w {
            self.scroll.0 = (x1 - w).max(0.0);
        }
        if y0 < self.scroll.1 {
            self.scroll.1 = y0;
        } else if y1 > self.scroll.1 + h {
            self.scroll.1 = (y1 - h).max(0.0);
        }
        self.ensure_subscribed();
    }

    pub(crate) fn addr(&self, row: u32, col: u32) -> Option<CellAddr> {
        Some(CellAddr::new(self.sheet?, row, col))
    }

    // -----------------------------------------------------------------------
    // Selection and navigation

    fn set_cursor(&mut self, row: u32, col: u32, extend: bool, cx: &mut Context<Self>) {
        if extend {
            self.sel.cursor = (row, col);
        } else {
            self.sel = Selection::at(row, col);
        }
        self.ensure_visible(row, col);
        self.tooltip = None;
        self.refresh_bar();
        cx.notify();
    }

    fn move_by(&mut self, dr: i64, dc: i64, extend: bool, cx: &mut Context<Self>) {
        let (r, c) = self.sel.cursor;
        let nr = (i64::from(r) + dr).clamp(0, i64::from(u32::MAX - 1)) as u32;
        let nc = (i64::from(c) + dc).clamp(0, i64::from(u32::MAX - 1)) as u32;
        if !extend {
            self.sel.mode = SelMode::Cells;
        }
        self.set_cursor(nr, nc, extend, cx);
    }

    fn jump(&mut self, dr: i64, dc: i64, extend: bool) {
        let Some(from) = self.addr(self.sel.cursor.0, self.sel.cursor.1) else {
            return;
        };
        self.request(Command::DataEdge { from, dr, dc }, move |this, r, _, cx| {
            if let Ok(Reply::Addresses(a)) = r
                && let Some(t) = a.first()
            {
                this.set_cursor(t.row, t.col, extend, cx);
            }
        });
    }

    fn page(&mut self, dir: i64, cx: &mut Context<Self>) {
        let vis = self.visible();
        let n = i64::from(vis.r2.saturating_sub(vis.r1).max(1));
        self.scroll.1 = (self.scroll.1 + dir as f64 * n as f64 * self.rows.default_size()).max(0.0);
        self.move_by(dir * n, 0, false, cx);
    }

    /// Loads the selected cell's edit text and hint into the edit bar.
    fn refresh_bar(&mut self) {
        let Some(addr) = self.addr(self.sel.cursor.0, self.sel.cursor.1) else {
            return;
        };
        if self.edit.is_some() {
            return;
        }
        self.request(Command::BeginEdit { addr }, move |this, r, _, cx| {
            if this.edit.is_none()
                && this.addr(this.sel.cursor.0, this.sel.cursor.1) == Some(addr)
                && let Ok(Reply::Edit { text, hint }) = r
            {
                this.bar_text = text;
                this.hint = hint;
                cx.notify();
            }
        });
    }

    // -----------------------------------------------------------------------
    // Editing

    fn begin_edit(&mut self, initial: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit.is_some() || self.info.as_ref().is_some_and(|i| i.read_only) {
            return;
        }
        let Some(addr) = self.addr(self.sel.cursor.0, self.sel.cursor.1) else {
            return;
        };
        self.edit_error = None;
        match initial {
            Some(text) => self.open_editor(addr, text, window, cx),
            None => self.request(Command::BeginEdit { addr }, move |this, r, window, cx| {
                if let Ok(Reply::Edit { text, hint }) = r {
                    this.hint = hint;
                    this.open_editor(addr, text, window, cx);
                }
            }),
        }
    }

    fn open_editor(
        &mut self,
        addr: CellAddr,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        trace(|| format!("open_editor {addr} {text:?}"));
        let len = text.encode_utf16().count() as u32;
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(text.clone(), window, cx);
            s.set_cursor_position(Position::new(0, len), window, cx);
            s
        });
        let sub = cx.subscribe_in(
            &input,
            window,
            |this, _input, e: &InputEvent, window, cx| {
                trace(|| {
                    format!(
                        "input event {}",
                        match e {
                            InputEvent::Blur => "blur",
                            InputEvent::Focus => "focus",
                            InputEvent::Change => "change",
                            _ => "other",
                        }
                    )
                });
                if let InputEvent::Blur = e
                    && this.edit.is_some()
                    && !this.focus.contains_focused(window, cx)
                {
                    // Focus moved elsewhere (another widget): commit.
                    this.commit_edit(0, 0, window, cx);
                }
            },
        );
        input.update(cx, |s, cx| s.focus(window, cx));
        self.bar_text = text;
        self.edit = Some(EditState {
            addr,
            input,
            _sub: sub,
        });
        cx.notify();
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        trace(|| "cancel_edit".into());
        self.edit = None;
        self.edit_error = None;
        self.focus.focus(window, cx);
        self.refresh_bar();
        cx.notify();
    }

    /// Commits the editor and moves the cursor by `(dr, dc)`.
    fn commit_edit(&mut self, dr: i64, dc: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.edit.take() else { return };
        let text = edit.input.read(cx).value().to_string();
        trace(|| format!("commit_edit {} {text:?}", edit.addr));
        let addr = edit.addr;
        // Optimistic display until the engine's update arrives; restored if
        // the edit is rejected.
        let previous = self.cache.get(&addr).cloned();
        let entry = self.cache.entry(addr).or_default();
        entry.text = text.clone();
        self.focus.focus(window, cx);
        self.request(
            Command::EnterText {
                addr,
                text: text.clone(),
            },
            move |this, r, window, cx| match r {
                Ok(Reply::Rejected(msg)) => {
                    match previous {
                        Some(d) => this.cache.insert(addr, d),
                        None => this.cache.remove(&addr),
                    };
                    this.sel = Selection::at(addr.row, addr.col);
                    this.open_editor(addr, text, window, cx);
                    this.edit_error = Some(msg);
                    this.refresh_bar_now(addr);
                }
                Err(e) => this.banner = Some(Banner::Error(e.to_string())),
                Ok(_) => {}
            },
        );
        self.edit_error = None;
        self.move_by(dr, dc, false, cx);
    }

    fn refresh_bar_now(&mut self, addr: CellAddr) {
        self.request(Command::BeginEdit { addr }, |this, r, _, cx| {
            if let Ok(Reply::Edit { hint, .. }) = r {
                this.hint = hint;
                cx.notify();
            }
        });
    }

    /// Returns whether the key was consumed.
    fn on_editor_key(
        &mut self,
        k: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        trace(|| format!("editor key {:?}", k.key));
        let shift = k.modifiers.shift;
        match k.key.as_str() {
            "enter" if !k.modifiers.control && !k.modifiers.alt => {
                self.commit_edit(if shift { -1 } else { 1 }, 0, window, cx);
                true
            }
            "tab" => {
                self.commit_edit(0, if shift { -1 } else { 1 }, window, cx);
                true
            }
            "escape" => {
                self.cancel_edit(window, cx);
                true
            }
            _ => false,
        }
    }

    fn on_grid_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.fps.as_mut() {
            f.key_at = Some(Instant::now());
        }
        let k = &e.keystroke;
        if k.key == PEEK_KEY && !k.modifiers.control && !k.modifiers.alt && self.edit.is_none() {
            if !self.peek {
                self.peek = true;
                self.load_legend();
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        if std::env::var("DECEL_DEBUG_KEYS").is_ok() {
            eprintln!(
                "grid key: {:?} char={:?} editing={}",
                k.key,
                k.key_char,
                self.edit.is_some()
            );
        }
        if self.edit.is_some() || k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return;
        }
        if let Some(ch) = &k.key_char
            && !ch.is_empty()
            && !ch.chars().any(char::is_control)
        {
            self.begin_edit(Some(ch.clone()), window, cx);
            cx.stop_propagation();
        }
    }

    fn clear_cells(&mut self) {
        let Some(sheet) = self.sheet else { return };
        let rect = self.sel.rect();
        let cells: Vec<(CellAddr, Cell)> = self
            .cache
            .iter()
            .filter(|(a, _)| a.sheet == sheet && rect.contains(a.row, a.col))
            .map(|(a, _)| (*a, Cell::EMPTY))
            .collect();
        // Clearing keeps metatables only when removing values from a single
        // computed cell would be pointless; Delete clears both.
        if cells.is_empty() {
            return;
        }
        let n = cells.len();
        self.send(Command::SetCells {
            label: format!("Clear {n} cell(s)"),
            cells,
        });
    }

    // -----------------------------------------------------------------------
    // Metatable tooling (Phase 5)

    /// `Ctrl+Enter` commits, `Esc` reverts the selected member.
    fn on_dock_key(
        &mut self,
        k: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match k.key.as_str() {
            "enter" if k.modifiers.control => {
                self.commit_meta(window, cx);
                true
            }
            "escape" => {
                self.revert_member(window, cx);
                true
            }
            _ => false,
        }
    }

    fn load_legend(&mut self) {
        let Some(sheet) = self.sheet else { return };
        self.request(Command::MetaSummary { sheet }, |this, r, _, cx| {
            if let Ok(Reply::Summary(s)) = r {
                this.legend = Some(s);
                cx.notify();
            }
        });
    }

    fn on_key_up(&mut self, e: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if e.keystroke.key == PEEK_KEY && self.peek {
            self.peek = false;
            cx.notify();
        }
    }

    fn on_modifiers(
        &mut self,
        e: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shift = e.modifiers.shift;
        if !self.shift && self.share_hl.is_some() {
            self.share_hl = None;
            cx.notify();
        }
    }

    fn toggle_code_side(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.code_side = !self.code_side;
        self.flip = Some(Instant::now());
        if self.code_side {
            self.load_legend();
            self.dock.open = true;
            self.dock_follow_selection(window, cx);
        } else {
            self.share_hl = None;
        }
        cx.notify();
    }

    /// Progress of the flip animation (`None` when not flipping).
    pub(crate) fn flip_progress(&self) -> Option<f32> {
        let t = self.flip?.elapsed().as_secs_f32() / FLIP.as_secs_f32();
        (t < 1.0).then_some(t)
    }

    fn highlight_share(
        &mut self,
        hash: Option<MetaHash>,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(h) = hash else {
            if self.share_hl.take().is_some() {
                cx.notify();
            }
            return;
        };
        if let Some(hl) = self.share_hl.as_mut()
            && hl.hash == h
        {
            hl.at = at;
            cx.notify();
            return;
        }
        let sheet = self.sheet;
        self.shift = true;
        self.request(Command::QueryShareSet { hash: h }, move |this, r, _, cx| {
            if !this.shift || !this.code_side {
                return;
            }
            if let Ok(Reply::ShareSet(cells)) = r {
                let other_sheets = cells.iter().filter(|a| Some(a.sheet) != sheet).count();
                this.share_hl = Some(ShareHighlight {
                    hash: h,
                    total: cells.len(),
                    other_sheets,
                    cells: cells.into_iter().collect(),
                    at,
                });
                cx.notify();
            }
        });
    }

    /// The fill handle: a small square at the selection's bottom-right.
    pub(crate) fn fill_handle(&self) -> Option<Point<Pixels>> {
        if self.sel.mode != SelMode::Cells || self.code_side {
            return None;
        }
        let r = self.sel.rect();
        let b = self.cell_bounds(r.r2, r.c2);
        Some(point(b.origin.x + b.size.width, b.origin.y + b.size.height))
    }

    fn on_fill_handle(&self, p: Point<Pixels>) -> bool {
        self.fill_handle()
            .is_some_and(|h| (p.x - h.x).abs() <= px(5.) && (p.y - h.y).abs() <= px(5.))
    }

    fn fill(&mut self, kind: FillKind, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet else { return };
        let dst = self.sel.rect();
        if dst.rows() * dst.cols() > 1_000_000 {
            return;
        }
        self.send(Command::Fill {
            kind,
            sheet,
            src: dst,
            dst,
            with_values: false,
            preview: false,
        });
        cx.notify();
    }

    fn auto_fill(&mut self, axis: Axis, cx: &mut Context<Self>) {
        let Some(addr) = self.addr(self.sel.cursor.0, self.sel.cursor.1) else {
            return;
        };
        self.request(
            Command::AutoFill {
                addr,
                axis,
                preview: true,
            },
            move |this, r, _, cx| {
                if let Ok(Reply::Addresses(cells)) = r {
                    this.autofill =
                        (!cells.is_empty()).then_some(AutoFillPreview { addr, axis, cells });
                    if this.autofill.is_none() {
                        this.status.last = "Auto-fill: nothing to fill".into();
                    }
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    fn apply_autofill(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.autofill.take() {
            self.send(Command::AutoFill {
                addr: p.addr,
                axis: p.axis,
                preview: false,
            });
            cx.notify();
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.autofill = None;
        self.fill_drag = None;
        self.share_hl = None;
        self.show_recent = false;
        cx.notify();
    }

    /// The selection clipped to cached data (whole rows/columns are huge).
    fn copy_rect(&self) -> Option<Rect> {
        let sheet = self.sheet?;
        let r = self.sel.rect();
        let (mut max_r, mut max_c) = (r.r1, r.c1);
        for a in self.cache.keys().filter(|a| a.sheet == sheet) {
            max_r = max_r.max(a.row);
            max_c = max_c.max(a.col);
        }
        Some(Rect::new(r.r1, r.c1, r.r2.min(max_r), r.c2.min(max_c)))
    }

    fn copy(&mut self, cut: bool, _cx: &mut Context<Self>) {
        let (Some(sheet), Some(rect)) = (self.sheet, self.copy_rect()) else {
            return;
        };
        self.request(Command::Copy { sheet, rect }, move |this, r, _, cx| {
            if let Ok(Reply::Clip(clip)) = r {
                cx.write_to_clipboard(ClipboardItem::new_string(clip.text.clone()));
                this.status.last = format!("Copied {} × {}", clip.rows, clip.cols);
                this.clip = Some(clip);
                if cut {
                    this.clear_cells();
                }
                cx.notify();
            }
        });
    }

    fn paste(&mut self, mode: PasteMode, cx: &mut Context<Self>) {
        let Some(at) = self.addr(self.sel.rect().r1, self.sel.rect().c1) else {
            return;
        };
        let text = cx.read_from_clipboard().and_then(|i| i.text());
        let internal = self
            .clip
            .as_ref()
            .filter(|c| text.is_none() || text.as_deref() == Some(c.text.as_str()))
            .cloned();
        match (internal, text) {
            (Some(clip), _) => {
                self.send(Command::Paste { at, clip, mode });
            }
            (None, Some(text)) if mode != PasteMode::Meta => {
                self.send(Command::PasteText { at, text });
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Files

    pub fn new_file(
        &mut self,
        path: Option<PathBuf>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let path = match path {
            Some(p) => p,
            None => match recent::new_untitled_path() {
                Ok(p) => p,
                Err(e) => {
                    self.banner = Some(Banner::Error(format!("cannot create workbook: {e}")));
                    return;
                }
            },
        };
        self.banner = None;
        self.sheet = None;
        self.request(Command::New { path: Some(path) }, |this, r, _, cx| {
            if let Err(e) = r {
                this.banner = Some(Banner::Error(e.to_string()));
                cx.notify();
            }
        });
    }

    pub fn open_path(
        &mut self,
        path: PathBuf,
        read_only: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.banner = None;
        self.sheet = None;
        let p = path.clone();
        self.request(Command::Open { path, read_only }, move |this, r, _, cx| {
            match r {
                Ok(_) => {}
                Err(CommandError::Locked { pid, host }) => {
                    let who = match (pid, host) {
                        (Some(pid), Some(h)) => format!("process {pid} on {h}"),
                        _ => "another decel".into(),
                    };
                    this.banner = Some(Banner::Locked { path: p, who });
                }
                Err(e) => this.banner = Some(Banner::Error(e.to_string())),
            }
            cx.notify();
        });
    }

    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open workbook".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(p) = paths.into_iter().next() {
                        this.open_path(p, false, window, cx);
                    }
                }
                Ok(Err(e)) => {
                    this.banner = Some(Banner::Error(format!("file picker failed: {e}")));
                    cx.notify();
                }
                _ => {}
            });
        })
        .detach();
    }

    fn prompt_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .info
            .as_ref()
            .and_then(|i| i.path.clone())
            .filter(|p| !recent::is_untitled(p))
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some("workbook.decel"));
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(Ok(Some(mut path))) => {
                    if path.extension().is_none() {
                        path.set_extension("decel");
                    }
                    this.request(Command::SaveAs { path }, |this, r, window, cx| {
                        if let Err(e) = r {
                            this.banner = Some(Banner::Error(e.to_string()));
                        }
                        this.update_title(window);
                        cx.notify();
                    });
                    let _ = window;
                }
                Ok(Err(e)) => {
                    this.banner = Some(Banner::Error(format!("file picker failed: {e}")));
                    cx.notify();
                }
                _ => {}
            });
        })
        .detach();
    }

    // -----------------------------------------------------------------------
    // Mouse

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.button != MouseButton::Left {
            return;
        }
        if self.edit.is_some() {
            self.commit_edit(0, 0, window, cx);
        }
        self.focus.focus(window, cx);
        self.tooltip = None;
        self.show_recent = false;
        if self.charts.selected.is_some() {
            self.select_chart(None, window, cx);
        }
        match self.hit(e.position) {
            Hit::ColHeader {
                edge: Some(idx), ..
            } => {
                if e.click_count >= 2 {
                    self.autofit_col(idx, window, cx);
                } else {
                    self.resize = Some(Resize::Col {
                        idx,
                        start: f64::from(e.position.x),
                        origin: self.cols.size(idx),
                    });
                }
            }
            Hit::RowHeader {
                edge: Some(idx), ..
            } => {
                self.resize = Some(Resize::Row {
                    idx,
                    start: f64::from(e.position.y),
                    origin: self.rows.size(idx),
                });
            }
            Hit::ColHeader { col, .. } => {
                if e.modifiers.shift && self.sel.mode == SelMode::Cols {
                    self.sel.cursor.1 = col;
                } else {
                    self.sel = Selection {
                        anchor: (self.visible().r1, col),
                        cursor: (self.visible().r1, col),
                        mode: SelMode::Cols,
                    };
                }
                self.dragging = true;
            }
            Hit::RowHeader { row, .. } => {
                if e.modifiers.shift && self.sel.mode == SelMode::Rows {
                    self.sel.cursor.0 = row;
                } else {
                    self.sel = Selection {
                        anchor: (row, self.visible().c1),
                        cursor: (row, self.visible().c1),
                        mode: SelMode::Rows,
                    };
                }
                self.dragging = true;
            }
            Hit::Cell { .. } if self.on_fill_handle(e.position) => {
                self.fill_drag = Some(self.sel.rect());
            }
            Hit::Cell { row, col } if self.code_side => {
                self.sel = Selection::at(row, col);
                if let Some(addr) = self.addr(row, col) {
                    let mode = if e.modifiers.shift {
                        MetaEditMode::Shared
                    } else {
                        MetaEditMode::Cow
                    };
                    self.dock_target(addr, mode, window, cx);
                }
            }
            Hit::Cell { row, col } => {
                if e.modifiers.shift {
                    self.sel.cursor = (row, col);
                } else {
                    self.sel = Selection::at(row, col);
                }
                self.dragging = true;
                self.refresh_bar();
                if e.click_count >= 2 {
                    self.dragging = false;
                    self.begin_edit(None, window, cx);
                }
            }
            Hit::Corner | Hit::Outside => {}
        }
        self.refresh_bar();
        cx.notify();
    }

    fn on_mouse_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = self.charts.drag.as_mut() {
            if e.pressed_button == Some(MouseButton::Left) {
                d.delta = e.position - d.start;
                cx.notify();
                return;
            }
            self.finish_chart_drag();
        }
        if let Some(r) = self.resize {
            match r {
                Resize::Col { idx, start, origin } => {
                    let w = (origin + f64::from(e.position.x) - start).max(12.0);
                    self.set_size_local(Axis::Col, idx, w);
                }
                Resize::Row { idx, start, origin } => {
                    let h = (origin + f64::from(e.position.y) - start).max(8.0);
                    self.set_size_local(Axis::Row, idx, h);
                }
            }
            cx.notify();
            return;
        }
        let hit = self.hit(e.position);
        let resize_hover = matches!(
            hit,
            Hit::ColHeader { edge: Some(_), .. } | Hit::RowHeader { edge: Some(_), .. }
        );
        if resize_hover != self.resize_hover {
            self.resize_hover = resize_hover;
            cx.notify();
        }
        if self.fill_drag.is_some() && e.pressed_button == Some(MouseButton::Left) {
            if let Hit::Cell { row, col } = hit {
                self.fill_drag = Some(fill_target(self.sel.rect(), row, col));
                cx.notify();
            }
            return;
        }
        if self.dragging && e.pressed_button == Some(MouseButton::Left) {
            self.drag_to(e.position, cx);
            return;
        }
        if self.code_side && e.modifiers.shift {
            let hash = match hit {
                Hit::Cell { row, col } => self
                    .addr(row, col)
                    .and_then(|a| self.cache.get(&a))
                    .and_then(|d| d.hash),
                _ => None,
            };
            self.highlight_share(hash, e.position, cx);
        } else if self.share_hl.is_some() {
            self.shift = e.modifiers.shift;
            self.share_hl = None;
            cx.notify();
        }
        if self.code_side {
            // Value tooltips are noise on the code side.
            self.hover = None;
            return;
        }
        // Hover tooltips.
        let cell = match hit {
            Hit::Cell { row, col } => self.addr(row, col),
            _ => None,
        };
        if cell != self.hover {
            self.hover = cell;
            if self.tooltip.is_some() {
                self.tooltip = None;
                cx.notify();
            }
            self._hover_task = cell.map(|addr| {
                let pos = e.position;
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(TOOLTIP_DELAY).await;
                    let _ = this.update(cx, |this, _cx| {
                        if this.hover == Some(addr) {
                            this.request(Command::Tooltip { addr }, move |this, r, _, cx| {
                                if this.hover == Some(addr)
                                    && let Ok(Reply::Tooltip(Some(t))) = r
                                {
                                    this.tooltip = Some((addr, t, pos));
                                    cx.notify();
                                }
                            });
                        }
                    });
                })
            });
        }
    }

    fn drag_to(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let b = self.grid_bounds.get();
        // Auto-scroll when dragging past an edge.
        let step = 0.5;
        if pos.y > b.origin.y + b.size.height {
            self.scroll.1 += self.rows.default_size() * step;
        } else if pos.y < b.origin.y + px(HEADER_H as f32) {
            self.scroll.1 = (self.scroll.1 - self.rows.default_size() * step).max(0.0);
        }
        if pos.x > b.origin.x + b.size.width {
            self.scroll.0 += self.cols.default_size() * step;
        } else if pos.x < b.origin.x + px(HEADER_W as f32) {
            self.scroll.0 = (self.scroll.0 - self.cols.default_size() * step).max(0.0);
        }
        let clamped = point(
            pos.x.clamp(
                b.origin.x + px(HEADER_W as f32 + 1.0),
                b.origin.x + b.size.width - px(1.0),
            ),
            pos.y.clamp(
                b.origin.y + px(HEADER_H as f32 + 1.0),
                b.origin.y + b.size.height - px(1.0),
            ),
        );
        if let Hit::Cell { row, col } = self.hit(clamped) {
            match self.sel.mode {
                SelMode::Cells => self.sel.cursor = (row, col),
                SelMode::Rows => self.sel.cursor.0 = row,
                SelMode::Cols => self.sel.cursor.1 = col,
            }
        }
        self.ensure_subscribed();
        cx.notify();
    }

    fn on_mouse_up(&mut self, e: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.dragging = false;
        if self.charts.drag.is_some() {
            self.finish_chart_drag();
            cx.notify();
            return;
        }
        if let (Some(dst), Some(sheet)) = (self.fill_drag.take(), self.sheet) {
            let src = self.sel.rect();
            if dst != src {
                self.send(Command::Fill {
                    kind: FillKind::Drag,
                    sheet,
                    src,
                    dst,
                    with_values: e.modifiers.control,
                    preview: false,
                });
                self.sel.anchor = (dst.r1, dst.c1);
                self.sel.cursor = (dst.r2, dst.c2);
            }
        }
        if let (Some(r), Some(sheet)) = (self.resize.take(), self.sheet) {
            let (axis, idx) = match r {
                Resize::Col { idx, .. } => (Axis::Col, idx),
                Resize::Row { idx, .. } => (Axis::Row, idx),
            };
            let size = match axis {
                Axis::Col => self.cols.size(idx),
                Axis::Row => self.rows.size(idx),
            };
            self.send(Command::SetLayout {
                sheet,
                axis,
                idx,
                size: Some(size),
            });
        }
        cx.notify();
    }

    fn on_scroll(&mut self, e: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let d = e.delta.pixel_delta(px(DEFAULT_ROW_H as f32));
        let (dx, dy) = if e.modifiers.shift && f32::from(d.x) == 0.0 {
            (f64::from(d.y), 0.0)
        } else {
            (f64::from(d.x), f64::from(d.y))
        };
        self.scroll.0 = (self.scroll.0 - dx).max(0.0);
        self.scroll.1 = (self.scroll.1 - dy).max(0.0);
        self.tooltip = None;
        self.ensure_subscribed();
        cx.notify();
    }

    /// Updates a size locally while dragging.
    fn set_size_local(&mut self, axis: Axis, idx: u32, size: f64) {
        let Some(info) = self.info.as_mut() else {
            return;
        };
        let Some(sheet) = info.sheets.iter_mut().find(|s| Some(s.id) == self.sheet) else {
            return;
        };
        match axis {
            Axis::Col => {
                sheet.col_widths.insert(idx, size);
                self.cols = AxisSizes::new(DEFAULT_COL_W, &sheet.col_widths);
            }
            Axis::Row => {
                sheet.row_heights.insert(idx, size);
                self.rows = AxisSizes::new(DEFAULT_ROW_H, &sheet.row_heights);
            }
        }
    }

    /// Double-click on a column edge: fit the widest cached text.
    fn autofit_col(&mut self, col: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet else { return };
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let mut widest = 0.0f64;
        for (a, d) in &self.cache {
            if a.sheet != sheet || a.col != col || d.text.is_empty() {
                continue;
            }
            let run = gpui::TextRun {
                len: d.text.len(),
                font: style.font(),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                SharedString::from(d.text.clone()),
                font_size,
                &[run],
                None,
            );
            widest = widest.max(f64::from(line.width()));
        }
        let w = (widest + 16.0).max(32.0);
        self.set_size_local(Axis::Col, col, w);
        self.send(Command::SetLayout {
            sheet,
            axis: Axis::Col,
            idx: col,
            size: Some(w),
        });
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Rendering

    fn render_edit_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (r, c) = self.sel.cursor;
        let text = match &self.edit {
            Some(e) => e.input.read(cx).value().to_string(),
            None => self.bar_text.clone(),
        };
        div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(28.))
                    .px_2()
                    .gap_3()
                    .child(
                        div()
                            .w(px(80.))
                            .text_color(theme.muted_foreground)
                            .child(addr_label(r, c)),
                    )
                    .child(div().flex_1().overflow_hidden().child(text))
                    .when_some(self.hint.clone(), |d, h| {
                        d.child(div().text_color(theme.muted_foreground).child(h))
                    }),
            )
            .when_some(self.edit_error.clone(), |d, msg| {
                d.child(div().px_2().pb_1().text_color(theme.danger).child(msg))
            })
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let banner = self.banner.as_ref()?;
        let theme = cx.theme();
        let base = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_3()
            .py_1()
            .bg(theme.warning.opacity(0.2))
            .border_b_1()
            .border_color(theme.border);
        Some(match banner {
            Banner::Locked { path, who } => {
                let p = path.clone();
                base.child(format!("{} is open in {who}.", path.display()))
                    .child(
                        div()
                            .id("open-ro")
                            .px_2()
                            .rounded_sm()
                            .bg(theme.secondary)
                            .cursor_pointer()
                            .child("Open read-only")
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.open_path(p.clone(), true, window, cx);
                            })),
                    )
                    .child(self.dismiss(cx))
            }
            Banner::ReadOnly => base
                .child("Read-only: this file is open elsewhere. Changes can't be saved.")
                .child(self.dismiss(cx)),
            Banner::Error(msg) => base.child(msg.clone()).child(self.dismiss(cx)),
        })
    }

    fn dismiss(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("dismiss-banner")
            .px_2()
            .cursor_pointer()
            .child("✕")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.banner = None;
                cx.notify();
            }))
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(28.))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.secondary);
        row = row.child(self.render_sheet_tabs(cx));
        let theme = cx.theme();
        let status = &self.status;
        let recalc = if status.pending > 0 {
            format!("⟳ {}", status.pending)
        } else {
            "⟳ idle".into()
        };
        let undo = match (&status.undo_label, status.can_undo) {
            (Some(l), true) => format!("Undo: {l}"),
            _ => "Nothing to undo".into(),
        };
        let redo = match (&status.redo_label, status.can_redo) {
            (Some(l), true) => format!("Redo: {l}"),
            _ => String::new(),
        };
        let fps = self.fps.as_ref().map(|f| {
            let key = f
                .key_latency
                .map(|d| format!(" · key→paint {:.1} ms", d.as_secs_f64() * 1000.0))
                .unwrap_or_default();
            format!("{:.0} fps{key}", f.fps())
        });
        let autofill = self.autofill.as_ref().map(|a| {
            format!(
                "Auto-fill {} cells — Enter to apply, Esc to cancel",
                a.cells.len()
            )
        });
        let last = (!status.last.is_empty()).then(|| status.last.clone());
        row.child(div().flex_1())
            .when_some(autofill, |d, t| {
                d.child(div().px_3().text_color(theme.warning).child(t))
            })
            .when_some(last.filter(|_| self.autofill.is_none()), |d, t| {
                d.child(div().px_3().text_color(theme.muted_foreground).child(t))
            })
            .when_some(fps, |d, f| d.child(div().px_3().child(f)))
            .child(div().px_3().text_color(theme.muted_foreground).child(undo))
            .when(!redo.is_empty(), |d| {
                d.child(div().px_3().text_color(theme.muted_foreground).child(redo))
            })
            .child(
                div()
                    .px_3()
                    .text_color(theme.muted_foreground)
                    .child(recalc),
            )
    }

    fn render_editor_overlay(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let edit = self.edit.as_ref()?;
        if edit.addr.sheet != self.sheet? {
            return None;
        }
        let b = self.cell_bounds(edit.addr.row, edit.addr.col);
        let g = self.grid_bounds.get();
        // Relative to the grid container.
        let left = b.origin.x - g.origin.x;
        let top = b.origin.y - g.origin.y;
        let theme = cx.theme();
        Some(
            div()
                .absolute()
                .left(left)
                .top(top)
                .min_w(b.size.width)
                .h(b.size.height)
                .bg(theme.background)
                .border_2()
                .border_color(theme.primary)
                .child(Input::new(&edit.input).appearance(false).h_full()),
        )
    }

    fn render_title(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("title")
            .flex()
            .flex_row()
            .gap_1()
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(cx.theme().secondary))
            .child(self.title())
            .child(div().text_color(cx.theme().muted_foreground).child("▾"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.show_recent = !this.show_recent;
                cx.notify();
            }))
    }

    /// Dropdown under the title: new, open, save as and recent files.
    fn render_recent(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.show_recent {
            return None;
        }
        let theme = cx.theme();
        let item = |id: SharedString| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .cursor_pointer()
                .hover(|s| s.bg(theme.secondary))
        };
        let mut list = div()
            .id("file-menu")
            .occlude()
            .absolute()
            .top(px(34.))
            .left(px(8.))
            .w(px(460.))
            .py_1()
            .rounded_md()
            .shadow_lg()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .text_color(theme.popover_foreground)
            .child(
                item("menu-new".into())
                    .child("New workbook   Ctrl+N")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.show_recent = false;
                        this.new_file(None, window, cx);
                    })),
            )
            .child(
                item("menu-open".into())
                    .child("Open…   Ctrl+O")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.show_recent = false;
                        this.prompt_open(window, cx);
                    })),
            )
            .child(
                item("menu-save".into())
                    .child("Save as…   Ctrl+Shift+S")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.show_recent = false;
                        this.prompt_save_as(window, cx);
                    })),
            )
            .child(
                item("menu-import".into())
                    .child("Import CSV…")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.show_recent = false;
                        this.prompt_import_csv(window, cx);
                    })),
            )
            .child(
                item("menu-export".into())
                    .child("Export CSV…")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.show_recent = false;
                        this.prompt_export_csv(window, cx);
                    })),
            )
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Recent"),
            );
        for (i, p) in recent::load().into_iter().enumerate() {
            let label = if recent::is_untitled(&p) {
                format!(
                    "Untitled ({})",
                    p.file_stem()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_default()
                )
            } else {
                p.display().to_string()
            };
            list = list.child(
                item(SharedString::from(format!("recent-{i}")))
                    .overflow_hidden()
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.show_recent = false;
                        this.open_path(p.clone(), false, window, cx);
                    })),
            );
        }
        Some(list)
    }

    /// Floating legend for peek and the code side: swatch, kind, unit, count.
    fn render_legend(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !(self.peek || self.code_side) {
            return None;
        }
        let legend = self.legend.as_ref()?;
        let theme = cx.theme();
        let dark = theme.is_dark();
        let colors = self.info.as_ref().map(|i| &i.meta_colors);
        let mut panel = div()
            .absolute()
            .right(px(16.))
            .bottom(px(16.))
            .max_w(px(320.))
            .p_2()
            .rounded_md()
            .shadow_lg()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap_1();
        if legend.is_empty() {
            panel = panel.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("No metatables on this sheet"),
            );
        }
        for item in legend.iter().take(24) {
            let idx = colors.and_then(|c| c.get(&item.hash)).copied().unwrap_or(0);
            let swatch = crate::palette::color(idx, dark);
            let name = item.kind.clone().unwrap_or_else(|| "(no kind)".into());
            let unit = item
                .unit
                .clone()
                .map(|u| format!(" [{u}]"))
                .unwrap_or_default();
            let count = if item.total > item.count {
                format!("{} (+{} elsewhere)", item.count, item.total - item.count)
            } else {
                item.count.to_string()
            };
            panel = panel.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(14.))
                            .h(px(14.))
                            .rounded_sm()
                            .bg(swatch)
                            .when(crate::palette::hatched(idx), |d| {
                                d.border_2().border_color(theme.foreground.opacity(0.5))
                            }),
                    )
                    .child(div().flex_1().child(format!("{name}{unit}")))
                    .child(div().text_color(theme.muted_foreground).child(count)),
            );
        }
        Some(panel)
    }

    fn render_share_label(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let hl = self.share_hl.as_ref()?;
        let g = self.grid_bounds.get();
        let theme = cx.theme();
        let text = if hl.other_sheets > 0 {
            format!(
                "{} cells ({} on other sheets) · Shift+click to edit all",
                hl.total, hl.other_sheets
            )
        } else {
            format!("{} cells · Shift+click to edit all", hl.total)
        };
        Some(
            div()
                .absolute()
                .left(hl.at.x - g.origin.x + px(14.))
                .top(hl.at.y - g.origin.y - px(28.))
                .px_2()
                .py_0p5()
                .rounded_md()
                .bg(theme.foreground)
                .text_color(theme.background)
                .text_xs()
                .child(text),
        )
    }

    fn render_tooltip(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (_, text, pos) = self.tooltip.as_ref()?;
        let g = self.grid_bounds.get();
        let theme = cx.theme();
        Some(
            div()
                .absolute()
                .left(pos.x - g.origin.x + px(12.))
                .top(pos.y - g.origin.y + px(16.))
                .max_w(px(420.))
                .px_2()
                .py_1()
                .rounded_md()
                .shadow_md()
                .bg(theme.popover)
                .border_1()
                .border_color(theme.border)
                .text_color(theme.popover_foreground)
                .child(text.clone()),
        )
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_subscribed();
        // The dock follows the selection (after this frame).
        let cursor_addr = self.addr(self.sel.cursor.0, self.sel.cursor.1);
        if self.dock.open && self.dock_tab == DockTab::History && self.history.addr != cursor_addr {
            self.refresh_history();
        }
        if self.dock.open
            && self.dock_tab == DockTab::Metatable
            && !self.dock_sync_pending
            && self.edit.is_none()
            && cursor_addr.is_some()
            && self.dock.target != cursor_addr
        {
            self.dock_sync_pending = true;
            cx.defer_in(window, |this, window, cx| {
                this.dock_sync_pending = false;
                this.dock_follow_selection(window, cx);
            });
        }
        if self.flip.is_some() && self.flip_progress().is_none() {
            self.flip = None;
        }
        if self.flip.is_some() {
            window.request_animation_frame();
        }
        let theme = cx.theme().clone();
        let cursor = if self.resize.is_some() || self.resize_hover {
            CursorStyle::ResizeLeftRight
        } else {
            CursorStyle::Arrow
        };
        let _ = window;
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_sm()
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(None, window, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.prompt_save_as(window, cx)))
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .on_action(
                cx.listener(|this, _: &ToggleCodeSide, window, cx| {
                    this.toggle_code_side(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleDock, window, cx| {
                if this.dock.open && this.dock_tab == DockTab::Metatable {
                    this.toggle_dock(window, cx);
                } else {
                    this.dock_tab = DockTab::Metatable;
                    this.dock.open = false;
                    this.toggle_dock(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &AddSheet, _, cx| this.add_sheet(cx)))
            .on_action(
                cx.listener(|this, _: &ImportCsv, window, cx| this.prompt_import_csv(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ExportCsv, window, cx| this.prompt_export_csv(window, cx)),
            )
            .on_drop(cx.listener(|this, p: &gpui::ExternalPaths, window, cx| {
                this.on_drop_paths(p, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &InsertCells, _, cx| this.structure_at_selection(false, cx)),
            )
            .on_action(
                cx.listener(|this, _: &DeleteCells, _, cx| this.structure_at_selection(true, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleHistory, _, cx| {
                if this.dock.open && this.dock_tab == DockTab::History {
                    this.dock.open = false;
                } else {
                    this.dock_tab = DockTab::History;
                    this.dock.open = true;
                    this.history.addr = None;
                    this.refresh_history();
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleBoundaries, _, cx| {
                this.boundaries = !this.boundaries;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NewMetaOnSelection, window, cx| {
                this.new_meta_on_selection(window, cx)
            }))
            .child(TitleBar::new().child(self.render_title(cx)))
            .when_some(self.render_banner(cx), |d, b| d.child(b))
            .child(self.render_edit_bar(cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .min_h(px(0.))
                    .child(
                        div()
                            .id("grid")
                            .h_full()
                            .relative()
                            .flex_1()
                            .overflow_hidden()
                            .key_context(GRID)
                            .track_focus(&self.focus)
                            .cursor(cursor)
                            .on_key_down(cx.listener(Self::on_grid_key))
                            .on_key_up(cx.listener(Self::on_key_up))
                            .on_modifiers_changed(cx.listener(Self::on_modifiers))
                            .on_action(cx.listener(|this, _: &FillDown, _, cx| {
                                this.fill(FillKind::Down, cx)
                            }))
                            .on_action(cx.listener(|this, _: &FillRight, _, cx| {
                                this.fill(FillKind::Right, cx)
                            }))
                            .on_action(cx.listener(|this, _: &AutoFillColumn, _, cx| {
                                this.auto_fill(Axis::Row, cx)
                            }))
                            .on_action(cx.listener(|this, _: &AutoFillRow, _, cx| {
                                this.auto_fill(Axis::Col, cx)
                            }))
                            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
                            .on_action(
                                cx.listener(|this, _: &CopyCells, _, cx| this.copy(false, cx)),
                            )
                            .on_action(cx.listener(|this, _: &CutCells, _, cx| this.copy(true, cx)))
                            .on_action(cx.listener(|this, _: &PasteCells, _, cx| {
                                this.paste(PasteMode::All, cx)
                            }))
                            .on_action(cx.listener(|this, _: &PasteValues, _, cx| {
                                this.paste(PasteMode::Values, cx)
                            }))
                            .on_action(cx.listener(|this, _: &PasteMeta, _, cx| {
                                this.paste(PasteMode::Meta, cx)
                            }))
                            .on_action(
                                cx.listener(|this, _: &MoveUp, _, cx| {
                                    this.move_by(-1, 0, false, cx)
                                }),
                            )
                            .on_action(cx.listener(|this, _: &MoveDown, _, cx| {
                                this.move_by(1, 0, false, cx)
                            }))
                            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| {
                                this.move_by(0, -1, false, cx)
                            }))
                            .on_action(cx.listener(|this, _: &MoveRight, _, cx| {
                                this.move_by(0, 1, false, cx)
                            }))
                            .on_action(cx.listener(|this, _: &SelectUp, _, cx| {
                                this.move_by(-1, 0, true, cx)
                            }))
                            .on_action(cx.listener(|this, _: &SelectDown, _, cx| {
                                this.move_by(1, 0, true, cx)
                            }))
                            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| {
                                this.move_by(0, -1, true, cx)
                            }))
                            .on_action(cx.listener(|this, _: &SelectRight, _, cx| {
                                this.move_by(0, 1, true, cx)
                            }))
                            .on_action(
                                cx.listener(|this, _: &JumpUp, _, _| this.jump(-1, 0, false)),
                            )
                            .on_action(
                                cx.listener(|this, _: &JumpDown, _, _| this.jump(1, 0, false)),
                            )
                            .on_action(
                                cx.listener(|this, _: &JumpLeft, _, _| this.jump(0, -1, false)),
                            )
                            .on_action(
                                cx.listener(|this, _: &JumpRight, _, _| this.jump(0, 1, false)),
                            )
                            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.page(-1, cx)))
                            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.page(1, cx)))
                            .on_action(cx.listener(|this, _: &RowStart, _, cx| {
                                let r = this.sel.cursor.0;
                                this.set_cursor(r, 0, false, cx)
                            }))
                            .on_action(cx.listener(|this, _: &RowEnd, _, _| {
                                let r = this.sel.cursor.0;
                                this.sel.cursor = (r, 0);
                                this.jump(0, 1, false)
                            }))
                            .on_action(cx.listener(|this, _: &SheetStart, _, cx| {
                                this.set_cursor(0, 0, false, cx)
                            }))
                            .on_action(cx.listener(|this, _: &EditCell, window, cx| {
                                if this.autofill.is_some() {
                                    this.apply_autofill(cx);
                                } else {
                                    this.begin_edit(None, window, cx)
                                }
                            }))
                            .on_action(cx.listener(|this, _: &ClearCells, window, cx| {
                                if let Some(id) = this.charts.selected {
                                    this.send(Command::ChartDelete { id });
                                    this.select_chart(None, window, cx);
                                } else {
                                    this.clear_cells()
                                }
                            }))
                            .on_action(
                                cx.listener(|this, _: &InsertChart, _, cx| this.insert_chart(cx)),
                            )
                            .on_action(cx.listener(|this, _: &Undo, _, _| {
                                this.send(Command::Undo);
                            }))
                            .on_action(cx.listener(|this, _: &Redo, _, _| {
                                this.send(Command::Redo);
                            }))
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                            .on_mouse_move(cx.listener(Self::on_mouse_move))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                            .on_scroll(cx.listener(Self::on_scroll))
                            .child(GridElement::new(cx.entity(), self.grid_bounds.clone()))
                            .children(self.render_charts(cx))
                            .when_some(self.render_editor_overlay(cx), |d, e| d.child(e))
                            .when_some(self.render_tooltip(cx), |d, t| d.child(t))
                            .when_some(self.render_legend(cx), |d, l| d.child(l))
                            .when_some(self.render_share_label(cx), |d, l| d.child(l)),
                    )
                    .when_some(self.render_dock(cx), |d, dock| d.child(dock)),
            )
            .child(self.render_tabs(cx))
            .when_some(self.render_recent(cx), |d, r| d.child(r))
            .when_some(self.render_csv_dialog(cx), |d, r| d.child(r))
    }
}
