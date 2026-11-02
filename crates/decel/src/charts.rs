// SPDX-License-Identifier: MIT OR Apache-2.0

//! Floating charts (DESIGN §8.6): data subscriptions, a small path-based
//! renderer (axes, nice ticks, gridlines, line/bar/scatter, legend), move and
//! resize with the mouse, and the Chart panel in the dock.

use std::collections::{BTreeMap, HashMap};

use decel_engine::protocol::Command;
use decel_model::{CellAddr, ChartId, ChartKind, ChartRecord, ChartSpec, RangeRef, Rect, Series};
use gpui::{
    App, AppContext as _, Bounds, ClickEvent, ContentMask, Context, Element, ElementId, Entity,
    GlobalElementId, Hsla, InspectorElementId, InteractiveElement as _, IntoElement, LayoutId,
    ParentElement as _, PathBuilder, Pixels, Point, SharedString, StatefulInteractiveElement as _,
    Style, Styled as _, TextAlign, TextRun, Window, div, fill, point, prelude::FluentBuilder as _,
    px, relative, size,
};
use gpui_component::ActiveTheme as _;
use gpui_component::input::{Input, InputState};

use crate::palette;
use crate::workspace::{HEADER_H, HEADER_W, Workspace};

/// Chart subscriptions use ids from here up (the grid uses 1).
const SUB_BASE: u32 = 1000;
const RESIZE_GRAB: f32 = 14.0;

#[derive(Clone, Copy)]
pub enum ChartDragKind {
    Move,
    Resize,
}

#[derive(Clone, Copy)]
pub struct ChartDrag {
    pub id: ChartId,
    pub kind: ChartDragKind,
    pub start: Point<Pixels>,
    pub delta: Point<Pixels>,
}

pub struct ChartInputs {
    pub title: Entity<InputState>,
    pub x_title: Entity<InputState>,
    pub y_title: Entity<InputState>,
}

#[derive(Default)]
pub struct ChartsState {
    /// Display text and numeric value of every cell the charts read.
    pub data: HashMap<CellAddr, (String, Option<f64>)>,
    /// Active chart subscriptions: id → range.
    pub subs: BTreeMap<u32, RangeRef>,
    pub selected: Option<ChartId>,
    pub drag: Option<ChartDrag>,
    pub inputs: Option<ChartInputs>,
}

/// One series ready to draw.
struct SeriesData {
    name: String,
    color: Hsla,
    /// `(x, y)`; for line/bar x is the category index.
    points: Vec<(f64, f64)>,
}

struct ChartData {
    spec: ChartSpec,
    categories: Vec<String>,
    series: Vec<SeriesData>,
    skipped: usize,
}

fn range_cells(r: Rect) -> Vec<(u32, u32)> {
    if r.rows().saturating_mul(r.cols()) > 100_000 {
        return Vec::new();
    }
    r.iter().collect()
}

fn hex_color(s: &str) -> Option<Hsla> {
    let s = s.strip_prefix('#')?;
    let v = u32::from_str_radix(s, 16).ok()?;
    (s.len() == 6).then(|| gpui::rgb(v).into())
}

impl Workspace {
    /// Keeps one subscription per chart range (charts update even when
    /// off-screen).
    pub(crate) fn sync_chart_subs(&mut self) {
        let mut want: BTreeMap<u32, RangeRef> = BTreeMap::new();
        if let Some(info) = &self.info {
            for (id, rec) in &info.charts {
                if let Some(spec) = ChartSpec::parse(&rec.spec) {
                    for (i, r) in spec.ranges().enumerate().take(64) {
                        want.insert(SUB_BASE + id.0 * 64 + i as u32, r);
                    }
                }
            }
        }
        let current = std::mem::take(&mut self.charts.subs);
        for (sub, r) in &current {
            if want.get(sub) != Some(r) {
                self.send(Command::Unsubscribe { id: *sub });
            }
        }
        for (sub, r) in &want {
            if current.get(sub) != Some(r) {
                self.send(Command::Subscribe {
                    id: *sub,
                    sheet: r.sheet,
                    rect: r.rect,
                });
            }
        }
        self.charts.subs = want;
    }

    /// Stores display updates for chart subscriptions. Returns whether `sub`
    /// was a chart subscription.
    pub(crate) fn on_chart_cells(
        &mut self,
        sub: u32,
        rect: Option<Rect>,
        full: bool,
        cells: &[(CellAddr, decel_engine::DisplayCell)],
    ) -> bool {
        let Some(range) = self.charts.subs.get(&sub).copied() else {
            return sub >= SUB_BASE;
        };
        if full && let Some(r) = rect {
            self.charts
                .data
                .retain(|a, _| a.sheet != range.sheet || !r.contains(a.row, a.col));
        }
        for (a, d) in cells {
            self.charts.data.insert(*a, (d.text.clone(), d.number));
        }
        true
    }

    fn chart_data(&self, rec: &ChartRecord, dark: bool) -> Option<ChartData> {
        let spec = ChartSpec::parse(&rec.spec)?;
        let get = |sheet, (r, c): (u32, u32)| self.charts.data.get(&CellAddr::new(sheet, r, c));
        let xs: Vec<(String, Option<f64>)> = match spec.x {
            Some(x) => range_cells(x.rect)
                .into_iter()
                .map(|rc| get(x.sheet, rc).cloned().unwrap_or_default())
                .collect(),
            None => Vec::new(),
        };
        let mut skipped = 0;
        let mut series = Vec::new();
        let mut n_points = 0;
        for (i, s) in spec.series.iter().enumerate() {
            let color = s
                .color
                .as_deref()
                .and_then(hex_color)
                .unwrap_or_else(|| palette::color(i as u32, dark));
            let mut points = Vec::new();
            for (k, rc) in range_cells(s.range.rect).into_iter().enumerate() {
                let (text, y) = get(s.range.sheet, rc).cloned().unwrap_or_default();
                let Some(y) = y.filter(|y| y.is_finite()) else {
                    if !text.is_empty() {
                        skipped += 1;
                    }
                    continue;
                };
                let x = match spec.kind {
                    ChartKind::Scatter => match xs.get(k).and_then(|x| x.1) {
                        Some(x) if x.is_finite() => x,
                        _ => {
                            if spec.x.is_some() {
                                skipped += 1;
                                continue;
                            }
                            k as f64 + 1.0
                        }
                    },
                    _ => k as f64,
                };
                points.push((x, y));
            }
            n_points = n_points.max(range_cells(s.range.rect).len());
            series.push(SeriesData {
                name: s.name.clone(),
                color,
                points,
            });
        }
        let categories = if xs.is_empty() {
            (1..=n_points).map(|i| i.to_string()).collect()
        } else {
            xs.into_iter().map(|(t, _)| t).collect()
        };
        Some(ChartData {
            spec,
            categories,
            series,
            skipped,
        })
    }

    /// Screen bounds of a chart (relative to the window), including any drag.
    pub(crate) fn chart_bounds(&self, rec: &ChartRecord) -> Option<Bounds<Pixels>> {
        if Some(rec.sheet) != self.sheet {
            return None;
        }
        let spec = ChartSpec::parse(&rec.spec).unwrap_or_default();
        let g = self.grid_bounds.get();
        let x = HEADER_W + self.cols.offset(rec.col) - self.scroll.0;
        let y = HEADER_H + self.rows.offset(rec.row) - self.scroll.1;
        let mut b = Bounds::new(
            point(g.origin.x + px(x as f32), g.origin.y + px(y as f32)),
            size(px(spec.width), px(spec.height)),
        );
        if let Some(d) = self.charts.drag
            && Some(d.id) == self.charts.selected
            && self.info.as_ref().and_then(|i| i.charts.get(&d.id)) == Some(rec)
        {
            match d.kind {
                ChartDragKind::Move => b.origin += d.delta,
                ChartDragKind::Resize => {
                    b.size.width = (b.size.width + d.delta.x).max(px(160.));
                    b.size.height = (b.size.height + d.delta.y).max(px(120.));
                }
            }
        }
        Some(b)
    }

    pub(crate) fn chart_at(&self, p: Point<Pixels>) -> Option<(ChartId, bool)> {
        let info = self.info.as_ref()?;
        for (id, rec) in info.charts.iter().rev() {
            let b = self.chart_bounds(rec)?;
            if b.contains(&p) {
                let corner = b.origin + point(b.size.width, b.size.height);
                let resize =
                    (corner.x - p.x) < px(RESIZE_GRAB) && (corner.y - p.y) < px(RESIZE_GRAB);
                return Some((*id, resize));
            }
        }
        None
    }

    pub(crate) fn select_chart(
        &mut self,
        id: Option<ChartId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.charts.selected = id;
        self.charts.inputs = None;
        if let Some(id) = id
            && let Some(rec) = self.info.as_ref().and_then(|i| i.charts.get(&id))
        {
            let spec = ChartSpec::parse(&rec.spec).unwrap_or_default();
            let mk = |text: String, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| {
                    let mut s = InputState::new(window, cx);
                    s.set_value(text, window, cx);
                    s
                })
            };
            self.charts.inputs = Some(ChartInputs {
                title: mk(spec.title, window, cx),
                x_title: mk(spec.x_title, window, cx),
                y_title: mk(spec.y_title, window, cx),
            });
            self.dock.open = true;
            self.dock_tab = crate::workspace::DockTab::Chart;
        } else if self.dock_tab == crate::workspace::DockTab::Chart {
            self.dock_tab = crate::workspace::DockTab::Metatable;
        }
        cx.notify();
    }

    /// Commits a change to a chart's spec (or anchor).
    pub(crate) fn update_chart(
        &mut self,
        id: ChartId,
        f: impl FnOnce(&mut ChartRecord, &mut ChartSpec),
    ) {
        let Some(mut rec) = self.info.as_ref().and_then(|i| i.charts.get(&id)).cloned() else {
            return;
        };
        let mut spec = ChartSpec::parse(&rec.spec).unwrap_or_default();
        f(&mut rec, &mut spec);
        rec.spec = spec.to_json();
        self.send(Command::ChartUpsert {
            id: Some(id),
            chart: rec,
        });
    }

    pub(crate) fn finish_chart_drag(&mut self) {
        let Some(d) = self.charts.drag.take() else {
            return;
        };
        if d.delta == Point::default() {
            return;
        }
        let Some(rec) = self
            .info
            .as_ref()
            .and_then(|i| i.charts.get(&d.id))
            .cloned()
        else {
            return;
        };
        match d.kind {
            ChartDragKind::Move => {
                let g = self.grid_bounds.get();
                let x = HEADER_W + self.cols.offset(rec.col) - self.scroll.0 + f64::from(d.delta.x);
                let y = HEADER_H + self.rows.offset(rec.row) - self.scroll.1 + f64::from(d.delta.y);
                let _ = g;
                let col = self.cols.index_at(x - HEADER_W + self.scroll.0 + 1.0);
                let row = self.rows.index_at(y - HEADER_H + self.scroll.1 + 1.0);
                self.update_chart(d.id, |r, _| {
                    r.row = row;
                    r.col = col;
                });
            }
            ChartDragKind::Resize => {
                let spec = ChartSpec::parse(&rec.spec).unwrap_or_default();
                let w = (spec.width + f32::from(d.delta.x)).max(160.0);
                let h = (spec.height + f32::from(d.delta.y)).max(120.0);
                self.update_chart(d.id, |_, s| {
                    s.width = w;
                    s.height = h;
                });
            }
        }
    }

    /// Inserts a chart from the selection: with two or more columns, the
    /// first is X and the rest are series; a non-numeric first row names them.
    pub(crate) fn insert_chart(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet else { return };
        let r = self.sel.rect();
        if r.rows() * r.cols() > 100_000 {
            return;
        }
        let text = |row: u32, col: u32| {
            self.cache
                .get(&CellAddr::new(sheet, row, col))
                .map(|d| (d.text.clone(), d.number))
                .unwrap_or_default()
        };
        let header = (r.c1..=r.c2).any(|c| {
            let (t, n) = text(r.r1, c);
            !t.is_empty() && n.is_none()
        }) && r.r2 > r.r1;
        let first = if header { r.r1 + 1 } else { r.r1 };
        let (x, cols) = if r.c2 > r.c1 {
            (
                Some(RangeRef {
                    sheet,
                    rect: Rect::new(first, r.c1, r.r2, r.c1),
                }),
                r.c1 + 1..=r.c2,
            )
        } else {
            (None, r.c1..=r.c2)
        };
        let series = cols
            .map(|c| Series {
                name: if header {
                    text(r.r1, c).0
                } else {
                    format!("Series {}", c - r.c1 + 1)
                },
                range: RangeRef {
                    sheet,
                    rect: Rect::new(first, c, r.r2, c),
                },
                color: None,
            })
            .collect();
        let spec = ChartSpec {
            x,
            series,
            ..ChartSpec::default()
        };
        self.send(Command::ChartUpsert {
            id: None,
            chart: ChartRecord {
                sheet,
                row: r.r1,
                col: r.c2.saturating_add(2),
                spec: spec.to_json(),
            },
        });
        cx.notify();
    }

    pub(crate) fn render_charts(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let Some(info) = &self.info else {
            return Vec::new();
        };
        let theme = cx.theme().clone();
        let g = self.grid_bounds.get();
        let mut out = Vec::new();
        for (id, rec) in &info.charts {
            let Some(b) = self.chart_bounds(rec) else {
                continue;
            };
            // Skip charts that are entirely off-screen.
            if b.origin.x > g.origin.x + g.size.width
                || b.origin.y > g.origin.y + g.size.height
                || b.origin.x + b.size.width < g.origin.x
                || b.origin.y + b.size.height < g.origin.y
            {
                continue;
            }
            let Some(data) = self.chart_data(rec, theme.is_dark()) else {
                continue;
            };
            let selected = self.charts.selected == Some(*id);
            let id = *id;
            out.push(
                div()
                    .id(SharedString::from(format!("chart-{}", id.0)))
                    .occlude()
                    .absolute()
                    .left(b.origin.x - g.origin.x)
                    .top(b.origin.y - g.origin.y)
                    .w(b.size.width)
                    .h(b.size.height)
                    .rounded_md()
                    .shadow_md()
                    .bg(theme.background)
                    .border_1()
                    .border_color(if selected {
                        theme.primary
                    } else {
                        theme.border
                    })
                    .when(selected, |d| d.border_2())
                    .child(ChartElement {
                        data,
                        text: theme.foreground,
                        muted: theme.muted_foreground,
                        grid: theme.border.opacity(0.7),
                        selected,
                    })
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if this.edit.is_some() {
                                return;
                            }
                            let resize = this.chart_at(e.position).is_some_and(|(_, r)| r);
                            if this.charts.selected != Some(id) {
                                this.select_chart(Some(id), window, cx);
                            }
                            this.charts.drag = Some(ChartDrag {
                                id,
                                kind: if resize {
                                    ChartDragKind::Resize
                                } else {
                                    ChartDragKind::Move
                                },
                                start: e.position,
                                delta: Point::default(),
                            });
                            this.focus.focus(window, cx);
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    pub(crate) fn render_chart_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut panel = div().p_3().flex().flex_col().gap_2();
        let (Some(id), Some(inputs)) = (self.charts.selected, self.charts.inputs.as_ref()) else {
            return panel.child("Select a chart, or select data and press Alt+F1 to insert one.");
        };
        let Some(spec) = self
            .info
            .as_ref()
            .and_then(|i| i.charts.get(&id))
            .and_then(|r| ChartSpec::parse(&r.spec))
        else {
            return panel;
        };
        let btn = |key: String, label: String, active: bool| {
            div()
                .id(SharedString::from(key))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .bg(if active {
                    theme.primary
                } else {
                    theme.secondary
                })
                .text_color(if active {
                    theme.primary_foreground
                } else {
                    theme.foreground
                })
                .child(label)
        };
        let mut kinds = div().flex().flex_row().gap_1();
        for (kind, label) in [
            (ChartKind::Line, "Line"),
            (ChartKind::Bar, "Bar"),
            (ChartKind::Scatter, "Scatter"),
        ] {
            kinds = kinds.child(
                btn(format!("kind-{label}"), label.into(), spec.kind == kind).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, _| {
                        this.update_chart(id, |_, s| s.kind = kind)
                    }),
                ),
            );
        }
        let field = |label: &'static str, input: &Entity<InputState>| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(70.))
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(div().flex_1().child(Input::new(input)))
        };
        panel = panel
            .child(kinds)
            .child(field("Title", &inputs.title))
            .child(field("X title", &inputs.x_title))
            .child(field("Y title", &inputs.y_title))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        btn("apply-titles".into(), "Apply titles".into(), false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let Some(inputs) = this.charts.inputs.as_ref() else {
                                    return;
                                };
                                let (t, xt, yt) = (
                                    inputs.title.read(cx).value().to_string(),
                                    inputs.x_title.read(cx).value().to_string(),
                                    inputs.y_title.read(cx).value().to_string(),
                                );
                                this.update_chart(id, |_, s| {
                                    s.title = t;
                                    s.x_title = xt;
                                    s.y_title = yt;
                                });
                            }),
                        ),
                    )
                    .child(
                        btn(
                            "legend".into(),
                            if spec.legend {
                                "Legend: on"
                            } else {
                                "Legend: off"
                            }
                            .into(),
                            spec.legend,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, _| {
                                this.update_chart(id, |_, s| s.legend = !s.legend)
                            },
                        )),
                    ),
            );
        let x_label = match spec.x {
            Some(r) => format!("X: {}", range_label(r)),
            None => "X: 1, 2, 3, …".into(),
        };
        panel = panel.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().flex_1().child(x_label))
                .child(
                    btn("x-sel".into(), "Use selection".into(), false).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, _| {
                            if let Some(r) = this.selection_range() {
                                this.update_chart(id, |_, s| s.x = Some(r));
                            }
                        },
                    )),
                )
                .child(
                    btn("x-none".into(), "None".into(), false).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, _| this.update_chart(id, |_, s| s.x = None),
                    )),
                ),
        );
        panel = panel.child(
            div()
                .pt_2()
                .text_color(theme.muted_foreground)
                .child("Series"),
        );
        let dark = theme.is_dark();
        for (i, s) in spec.series.iter().enumerate() {
            let color = s
                .color
                .as_deref()
                .and_then(hex_color)
                .unwrap_or_else(|| palette::color(i as u32, dark));
            panel = panel.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id(SharedString::from(format!("swatch-{i}")))
                            .w(px(14.))
                            .h(px(14.))
                            .rounded_sm()
                            .bg(color)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, _| {
                                // Cycle through the palette.
                                this.update_chart(id, |_, sp| {
                                    if let Some(s) = sp.series.get_mut(i) {
                                        let next = (i as u32
                                            + 1
                                            + s.color.as_ref().map_or(0, |c| c.len() as u32))
                                            % 16;
                                        let c = palette::color(next, false).to_rgb();
                                        s.color = Some(format!(
                                            "#{:02x}{:02x}{:02x}",
                                            (c.r * 255.0) as u8,
                                            (c.g * 255.0) as u8,
                                            (c.b * 255.0) as u8
                                        ));
                                    }
                                })
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(format!("{} — {}", s.name, range_label(s.range))),
                    )
                    .child(
                        btn(format!("s-sel-{i}"), "Use selection".into(), false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, _| {
                                if let Some(r) = this.selection_range() {
                                    this.update_chart(id, |_, sp| {
                                        if let Some(s) = sp.series.get_mut(i) {
                                            s.range = r;
                                        }
                                    });
                                }
                            }),
                        ),
                    )
                    .child(
                        btn(format!("s-rm-{i}"), "✕".into(), false).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, _| {
                                this.update_chart(id, |_, sp| {
                                    if i < sp.series.len() {
                                        sp.series.remove(i);
                                    }
                                })
                            },
                        )),
                    ),
            );
        }
        panel
            .child(
                btn("s-add".into(), "+ Add series from selection".into(), false).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, _| {
                        if let Some(r) = this.selection_range() {
                            this.update_chart(id, |_, sp| {
                                let n = sp.series.len() + 1;
                                sp.series.push(Series {
                                    name: format!("Series {n}"),
                                    range: r,
                                    color: None,
                                });
                            });
                        }
                    }),
                ),
            )
            .child(
                btn("chart-delete".into(), "Delete chart".into(), false).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.send(Command::ChartDelete { id });
                        this.select_chart(None, window, cx);
                    },
                )),
            )
    }

    fn selection_range(&self) -> Option<RangeRef> {
        let sheet = self.sheet?;
        let r = self.sel.rect();
        (r.rows() * r.cols() <= 100_000).then_some(RangeRef { sheet, rect: r })
    }
}

fn range_label(r: RangeRef) -> String {
    let rect = r.rect;
    format!(
        "{},{} : {},{}",
        u64::from(rect.r1) + 1,
        u64::from(rect.c1) + 1,
        u64::from(rect.r2) + 1,
        u64::from(rect.c2) + 1
    )
}

/// "Nice" axis ticks covering `[lo, hi]`.
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> (f64, f64, f64) {
    let (lo, hi) = if (hi - lo).abs() < f64::EPSILON {
        (lo - 1.0, hi + 1.0)
    } else {
        (lo, hi)
    };
    let raw = (hi - lo) / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    } * mag;
    ((lo / step).floor() * step, (hi / step).ceil() * step, step)
}

fn tick_label(v: f64, step: f64) -> String {
    let dp = if step >= 1.0 {
        0
    } else {
        (-step.log10().floor()) as usize
    };
    // Snap values within rounding error of zero (and -0.0) to 0.
    let v = if v.abs() < step * 1e-9 { 0.0 } else { v };
    format!("{v:.dp$}")
}

/// Maps an x value to a horizontal position in the plot.
type XMap = Box<dyn Fn(f64) -> Pixels>;

struct ChartElement {
    data: ChartData,
    text: Hsla,
    muted: Hsla,
    grid: Hsla,
    selected: bool,
}

impl IntoElement for ChartElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

fn label(window: &Window, text: &str, color: Hsla, small: bool) -> gpui::ShapedLine {
    let style = window.text_style();
    let mut font_size = style.font_size.to_pixels(window.rem_size());
    if small {
        font_size *= 0.85;
    }
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(
        SharedString::from(text.to_string()),
        font_size,
        &[run],
        None,
    )
}

impl Element for ChartElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let d = &self.data;
        let lh = window.line_height();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            let o = bounds.origin;
            let (w, h) = (bounds.size.width, bounds.size.height);
            // Title.
            let mut top = px(8.);
            if !d.spec.title.is_empty() {
                let t = label(window, &d.spec.title, self.text, false);
                let _ = t.paint(
                    point(o.x + (w - t.width()) / 2.0, o.y + px(6.)),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                top += lh + px(4.);
            }
            if !d.spec.y_title.is_empty() {
                let t = label(window, &d.spec.y_title, self.muted, true);
                let _ = t.paint(
                    point(o.x + px(8.), o.y + top - px(2.)),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                top += lh;
            }
            let legend_w = if d.spec.legend && !d.series.is_empty() {
                d.series
                    .iter()
                    .map(|s| label(window, &s.name, self.text, true).width())
                    .fold(px(0.), |a, b| a.max(b))
                    + px(28.)
            } else {
                px(0.)
            };
            let mut bottom = lh + px(10.);
            if !d.spec.x_title.is_empty() {
                bottom += lh;
            }
            // Y range.
            let ys = d.series.iter().flat_map(|s| s.points.iter().map(|p| p.1));
            let (mut lo, mut hi) = ys.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), y| {
                (a.min(y), b.max(y))
            });
            if !lo.is_finite() {
                (lo, hi) = (0.0, 1.0);
            }
            if d.spec.kind == ChartKind::Bar {
                lo = lo.min(0.0);
                hi = hi.max(0.0);
            }
            let (y0, y1, ystep) = nice_ticks(lo, hi, 5);
            let tick_w = (0..=((y1 - y0) / ystep).round() as i64)
                .map(|i| {
                    label(
                        window,
                        &tick_label(y0 + i as f64 * ystep, ystep),
                        self.muted,
                        true,
                    )
                    .width()
                })
                .fold(px(0.), |a, b| a.max(b));
            let plot = Bounds::from_corners(
                point(o.x + tick_w + px(14.), o.y + top + px(6.)),
                point(o.x + w - px(12.) - legend_w, o.y + h - bottom),
            );
            if plot.size.width <= px(10.) || plot.size.height <= px(10.) {
                return;
            }
            let ymap =
                |y: f64| plot.origin.y + plot.size.height * (1.0 - ((y - y0) / (y1 - y0)) as f32);
            // Gridlines and y ticks.
            let mut v = y0;
            while v <= y1 + ystep * 0.5 {
                let y = ymap(v);
                window.paint_quad(fill(
                    Bounds::new(point(plot.origin.x, y), size(plot.size.width, px(1.))),
                    self.grid,
                ));
                let t = label(window, &tick_label(v, ystep), self.muted, true);
                let _ = t.paint(
                    point(plot.origin.x - t.width() - px(6.), y - lh / 2.0),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                v += ystep;
            }
            // Axes.
            window.paint_quad(fill(
                Bounds::new(plot.origin, size(px(1.), plot.size.height)),
                self.muted,
            ));
            window.paint_quad(fill(
                Bounds::new(
                    point(plot.origin.x, plot.origin.y + plot.size.height),
                    size(plot.size.width, px(1.)),
                ),
                self.muted,
            ));
            // X mapping.
            let n = d.categories.len().max(1);
            let (xmap, xlabels): (XMap, Vec<(Pixels, String)>) = match d.spec.kind {
                ChartKind::Scatter => {
                    let xs = d.series.iter().flat_map(|s| s.points.iter().map(|p| p.0));
                    let (xl, xh) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
                        (a.min(x), b.max(x))
                    });
                    let (xl, xh) = if xl.is_finite() { (xl, xh) } else { (0.0, 1.0) };
                    let (x0, x1, xstep) = nice_ticks(xl, xh, 5);
                    let pw = plot.size.width;
                    let ox = plot.origin.x;
                    let map = move |x: f64| ox + pw * ((x - x0) / (x1 - x0)) as f32;
                    let mut labels = Vec::new();
                    let mut v = x0;
                    while v <= x1 + xstep * 0.5 {
                        labels.push((map(v), tick_label(v, xstep)));
                        v += xstep;
                    }
                    (Box::new(map), labels)
                }
                _ => {
                    let slot = plot.size.width / n as f32;
                    let ox = plot.origin.x;
                    let map = move |x: f64| ox + slot * (x as f32 + 0.5);
                    // Thin labels so they don't overlap.
                    let widest = d
                        .categories
                        .iter()
                        .map(|t| label(window, t, self.muted, true).width())
                        .fold(px(0.), |a, b| a.max(b));
                    let every =
                        ((f32::from(widest + px(8.)) / f32::from(slot)).ceil() as usize).max(1);
                    let labels = d
                        .categories
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i % every == 0)
                        .map(|(i, t)| (map(i as f64), t.clone()))
                        .collect();
                    (Box::new(map), labels)
                }
            };
            for (x, t) in &xlabels {
                let l = label(window, t, self.muted, true);
                let _ = l.paint(
                    point(
                        *x - l.width() / 2.0,
                        plot.origin.y + plot.size.height + px(4.),
                    ),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            if !d.spec.x_title.is_empty() {
                let t = label(window, &d.spec.x_title, self.muted, true);
                let _ = t.paint(
                    point(
                        plot.origin.x + (plot.size.width - t.width()) / 2.0,
                        o.y + h - lh - px(4.),
                    ),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            // Series.
            window.with_content_mask(Some(ContentMask { bounds: plot }), |window| {
                let ns = d.series.len().max(1) as f32;
                let slot = plot.size.width / n as f32;
                for (si, s) in d.series.iter().enumerate() {
                    match d.spec.kind {
                        ChartKind::Line => {
                            let mut path = PathBuilder::stroke(px(2.));
                            for (k, (x, y)) in s.points.iter().enumerate() {
                                let p = point(xmap(*x), ymap(*y));
                                if k == 0 {
                                    path.move_to(p);
                                } else {
                                    path.line_to(p);
                                }
                            }
                            if s.points.len() > 1
                                && let Ok(path) = path.build()
                            {
                                window.paint_path(path, s.color);
                            }
                            if s.points.len() <= 60 {
                                for (x, y) in &s.points {
                                    dot(window, point(xmap(*x), ymap(*y)), px(2.5), s.color);
                                }
                            }
                        }
                        ChartKind::Bar => {
                            let bw = (slot * 0.8 / ns).max(px(1.));
                            let zero = ymap(0.0_f64.clamp(y0, y1));
                            for (x, y) in &s.points {
                                let left = plot.origin.x
                                    + slot * (*x as f32)
                                    + slot * 0.1
                                    + bw * si as f32;
                                let yy = ymap(*y);
                                let (t, b) = if yy < zero { (yy, zero) } else { (zero, yy) };
                                window.paint_quad(fill(
                                    Bounds::from_corners(
                                        point(left, t),
                                        point(left + bw - px(1.), b),
                                    ),
                                    s.color,
                                ));
                            }
                        }
                        ChartKind::Scatter => {
                            for (x, y) in &s.points {
                                dot(window, point(xmap(*x), ymap(*y)), px(3.5), s.color);
                            }
                        }
                    }
                }
            });
            // Legend.
            if legend_w > px(0.) {
                let mut y = plot.origin.y;
                for s in &d.series {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(o.x + w - legend_w, y + lh / 2.0 - px(5.)),
                            size(px(10.), px(10.)),
                        ),
                        s.color,
                    ));
                    let t = label(window, &s.name, self.text, true);
                    let _ = t.paint(
                        point(o.x + w - legend_w + px(16.), y),
                        lh,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    y += lh;
                }
            }
            if d.skipped > 0 {
                let t = label(
                    window,
                    &format!("{} non-numeric skipped", d.skipped),
                    self.muted,
                    true,
                );
                let _ = t.paint(
                    point(o.x + w - t.width() - px(6.), o.y + h - lh - px(2.)),
                    lh,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            if self.selected {
                // Resize grip.
                let c = o + point(w, h);
                for i in 1..=3 {
                    let k = px(4. * i as f32);
                    let mut p = PathBuilder::stroke(px(1.));
                    p.move_to(point(c.x - k, c.y - px(2.)));
                    p.line_to(point(c.x - px(2.), c.y - k));
                    if let Ok(p) = p.build() {
                        window.paint_path(p, self.muted);
                    }
                }
            }
        });
    }
}

fn dot(window: &mut Window, c: Point<Pixels>, r: Pixels, color: Hsla) {
    let mut q = fill(
        Bounds::new(point(c.x - r, c.y - r), size(r * 2.0, r * 2.0)),
        color,
    );
    q.corner_radii = gpui::Corners::all(r);
    window.paint_quad(q);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_ticks_cover_range() {
        let (lo, hi, step) = nice_ticks(3.0, 97.0, 5);
        assert_eq!((lo, hi, step), (0.0, 100.0, 20.0));
        let (lo, hi, step) = nice_ticks(-0.3, 0.7, 5);
        assert_eq!(step, 0.2);
        assert!(lo <= -0.3 && hi >= 0.7);
        let (lo, hi, _) = nice_ticks(5.0, 5.0, 5);
        assert!(lo < 5.0 && hi > 5.0);
    }

    #[test]
    fn tick_labels() {
        assert_eq!(tick_label(20.0, 20.0), "20");
        assert_eq!(tick_label(0.4, 0.2), "0.4");
        assert_eq!(tick_label(-0.0, 0.5), "0.0");
    }
}
