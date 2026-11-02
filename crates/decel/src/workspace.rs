// SPDX-License-Identifier: MIT OR Apache-2.0

//! The main window view: title bar, edit bar, grid, sheet tabs and status
//! bar (DESIGN §8.1). All engine access goes through commands; replies and
//! events arrive asynchronously.

use std::cell::Cell as StdCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use decel_engine::protocol::{Command, Event, Reply, WorkbookInfo};
use decel_engine::{CommandError, DisplayCell, EngineHandle};
use decel_model::{Axis, Cell, CellAddr, Rect, SheetId};
use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    App, AppContext as _, Bounds, ClickEvent, Context, CursorStyle, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, PathPromptOptions, Pixels, Point, Render,
    ScrollEvent, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, point, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputEvent, InputState, Position};
use gpui_component::{ActiveTheme as _, TitleBar};

use crate::actions::*;
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

struct EditState {
    addr: CellAddr,
    input: Entity<InputState>,
    _sub: Subscription,
}

#[derive(Debug, Clone, Copy)]
enum Resize {
    Col { idx: u32, start: f64, origin: f64 },
    Row { idx: u32, start: f64, origin: f64 },
}

enum Banner {
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

pub enum Hit {
    Corner,
    ColHeader { col: u32, edge: Option<u32> },
    RowHeader { row: u32, edge: Option<u32> },
    Cell { row: u32, col: u32 },
    Outside,
}

pub struct Workspace {
    engine: EngineHandle,
    replies: HashMap<u64, ReplyFn>,
    pub focus: FocusHandle,
    pub info: Option<WorkbookInfo>,
    pub sheet: Option<SheetId>,
    pub cols: AxisSizes,
    pub rows: AxisSizes,
    pub cache: HashMap<CellAddr, DisplayCell>,
    subscribed: Option<(SheetId, Rect)>,
    /// Scroll offset of the body in content pixels (x, y).
    pub scroll: (f64, f64),
    pub sel: Selection,
    edit: Option<EditState>,
    /// Edit text of the selected cell (shown in the edit bar).
    bar_text: String,
    hint: Option<String>,
    edit_error: Option<String>,
    tooltip: Option<(CellAddr, String, Point<Pixels>)>,
    hover: Option<CellAddr>,
    _hover_task: Option<Task<()>>,
    resize: Option<Resize>,
    resize_hover: bool,
    dragging: bool,
    pub grid_bounds: Rc<StdCell<Bounds<Pixels>>>,
    pub status: Status,
    banner: Option<Banner>,
    show_recent: bool,
    pub fps: Option<FpsMeter>,
    _event_task: Task<()>,
    _keys: Subscription,
}

fn trace(msg: impl FnOnce() -> String) {
    if std::env::var("DECEL_DEBUG_KEYS").is_ok() {
        eprintln!("{}", msg());
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
            if ws.read(cx).edit.is_none() {
                return;
            }
            let handled = ws.update(cx, |ws, cx| ws.on_editor_key(&ev.keystroke, window, cx));
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
        };
        match open {
            Some(p) if p.exists() => this.open_path(p, false, window, cx),
            Some(p) => this.new_file(Some(p), window, cx),
            None => this.new_file(None, window, cx),
        }
        this.focus.focus(window, cx);
        this
    }

    // -----------------------------------------------------------------------
    // Engine plumbing

    fn send(&mut self, cmd: Command) -> u64 {
        self.engine.send(cmd)
    }

    fn request(
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
                sheet,
                rect,
                full,
                cells,
                ..
            } => {
                if Some(sheet) != self.sheet {
                    return;
                }
                if full && let Some(r) = rect {
                    self.cache
                        .retain(|a, _| a.sheet == sheet && r.contains(a.row, a.col));
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
                ..
            } => {
                self.status.can_undo = can_undo;
                self.status.can_redo = can_redo;
                self.status.undo_label = undo_label;
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
        let sheet = match self.sheet {
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

    fn addr(&self, row: u32, col: u32) -> Option<CellAddr> {
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
        if self.dragging && e.pressed_button == Some(MouseButton::Left) {
            self.drag_to(e.position, cx);
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

    fn on_mouse_up(&mut self, _e: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.dragging = false;
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
        if let Some(info) = &self.info {
            for s in &info.sheets {
                let id = s.id;
                let active = Some(id) == self.sheet;
                row = row.child(
                    div()
                        .id(SharedString::from(format!("tab-{}", id.0)))
                        .px_3()
                        .h_full()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .when(active, |d| {
                            d.bg(theme.background)
                                .border_b_2()
                                .border_color(theme.primary)
                        })
                        .child(s.name.clone())
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.switch_sheet(id, window, cx);
                        })),
                );
            }
        }
        let status = &self.status;
        let recalc = if status.pending > 0 {
            format!("⟳ {}", status.pending)
        } else {
            "⟳ idle".into()
        };
        let undo = match (&status.undo_label, status.can_undo) {
            (Some(l), true) => format!("Undo: {l}"),
            _ => String::new(),
        };
        let fps = self.fps.as_ref().map(|f| {
            let key = f
                .key_latency
                .map(|d| format!(" · key→paint {:.1} ms", d.as_secs_f64() * 1000.0))
                .unwrap_or_default();
            format!("{:.0} fps{key}", f.fps())
        });
        row.child(div().flex_1())
            .when_some(fps, |d, f| d.child(div().px_3().child(f)))
            .child(div().px_3().text_color(theme.muted_foreground).child(undo))
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
            .child(TitleBar::new().child(self.render_title(cx)))
            .when_some(self.render_banner(cx), |d, b| d.child(b))
            .child(self.render_edit_bar(cx))
            .child(
                div()
                    .id("grid")
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .key_context(GRID)
                    .track_focus(&self.focus)
                    .cursor(cursor)
                    .on_key_down(cx.listener(Self::on_grid_key))
                    .on_action(
                        cx.listener(|this, _: &MoveUp, _, cx| this.move_by(-1, 0, false, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &MoveDown, _, cx| this.move_by(1, 0, false, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &MoveLeft, _, cx| this.move_by(0, -1, false, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &MoveRight, _, cx| this.move_by(0, 1, false, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &SelectUp, _, cx| this.move_by(-1, 0, true, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &SelectDown, _, cx| this.move_by(1, 0, true, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &SelectLeft, _, cx| this.move_by(0, -1, true, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &SelectRight, _, cx| this.move_by(0, 1, true, cx)),
                    )
                    .on_action(cx.listener(|this, _: &JumpUp, _, _| this.jump(-1, 0, false)))
                    .on_action(cx.listener(|this, _: &JumpDown, _, _| this.jump(1, 0, false)))
                    .on_action(cx.listener(|this, _: &JumpLeft, _, _| this.jump(0, -1, false)))
                    .on_action(cx.listener(|this, _: &JumpRight, _, _| this.jump(0, 1, false)))
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
                    .on_action(
                        cx.listener(|this, _: &SheetStart, _, cx| this.set_cursor(0, 0, false, cx)),
                    )
                    .on_action(cx.listener(|this, _: &EditCell, window, cx| {
                        this.begin_edit(None, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &ClearCells, _, _| this.clear_cells()))
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
                    .when_some(self.render_editor_overlay(cx), |d, e| d.child(e))
                    .when_some(self.render_tooltip(cx), |d, t| d.child(t)),
            )
            .child(self.render_tabs(cx))
            .when_some(self.render_recent(cx), |d, r| d.child(r))
    }
}
