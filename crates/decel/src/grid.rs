// SPDX-License-Identifier: MIT OR Apache-2.0

//! The virtualised grid element (DESIGN §8.2, §8.3).
//!
//! Only visible cells are shaped and painted. Painting order: backgrounds
//! (and metatable tints in peek / code side), metatable boundaries, grid
//! lines, selection fill, text, markers, selection outline, fill handle,
//! overlays (fill/auto-fill previews, share-set outline), then the frozen
//! headers on top.
//!
//! The code-side flip scales the body horizontally through zero over
//! [`FLIP`](crate::workspace::FLIP); text is hidden while it runs.

use std::cell::Cell as StdCell;
use std::rc::Rc;

use decel_engine::{Align, DisplayCell};
use decel_model::{CellAddr, MetaHash, Rect};
use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Entity, FontStyle, FontWeight, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, PathBuilder, Pixels, Point, ShapedLine,
    SharedString, Style, TextAlign, TextRun, Window, fill, point, px, relative, rgb, size,
};
use gpui_component::ActiveTheme as _;

use crate::palette;
use crate::workspace::{HEADER_H, HEADER_W, SelMode, Workspace};

const PAD: f32 = 5.0;
const MARKER: f32 = 6.0;
const HANDLE: f32 = 6.0;

pub struct GridElement {
    view: Entity<Workspace>,
    bounds_out: Rc<StdCell<Bounds<Pixels>>>,
}

impl GridElement {
    pub fn new(view: Entity<Workspace>, bounds_out: Rc<StdCell<Bounds<Pixels>>>) -> Self {
        Self { view, bounds_out }
    }
}

struct Colors {
    bg: Hsla,
    header: Hsla,
    header_active: Hsla,
    line: Hsla,
    text: Hsla,
    muted: Hsla,
    header_text: Hsla,
    select: Hsla,
    select_fill: Hsla,
    error: Hsla,
    warning: Hsla,
    boundary: Hsla,
}

struct PaintCell {
    bounds: Bounds<Pixels>,
    bg: Option<Hsla>,
    /// Metatable tint and whether it is hatched (peek / code side).
    tint: Option<(Hsla, bool)>,
    /// Text clip (spilled into empty neighbours when the text overflows).
    clip: Bounds<Pixels>,
    line: Option<(ShapedLine, Point<Pixels>)>,
    marker: Option<Hsla>,
}

pub struct Prepared {
    colors: Colors,
    body: Bounds<Pixels>,
    cells: Vec<PaintCell>,
    boundaries: Vec<(Bounds<Pixels>, Hsla)>,
    hlines: Vec<Pixels>,
    vlines: Vec<Pixels>,
    col_heads: Vec<(Bounds<Pixels>, ShapedLine, bool)>,
    row_heads: Vec<(Bounds<Pixels>, ShapedLine, bool)>,
    sel: Option<Bounds<Pixels>>,
    cursor: Option<Bounds<Pixels>>,
    handle: Option<Bounds<Pixels>>,
    fill_preview: Option<Bounds<Pixels>>,
    autofill: Vec<Bounds<Pixels>>,
    share: Vec<Bounds<Pixels>>,
    share_color: Hsla,
    animating: bool,
}

fn rgb_hsla((r, g, b): (u8, u8, u8)) -> Hsla {
    rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)).into()
}

fn shape(
    window: &Window,
    text: &str,
    color: Hsla,
    d: Option<&DisplayCell>,
    mono_family: &SharedString,
) -> ShapedLine {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let mut font = style.font();
    if let Some(d) = d {
        if d.style.bold == Some(true) {
            font.weight = FontWeight::BOLD;
        }
        if d.style.italic == Some(true) {
            font.style = FontStyle::Italic;
        }
        if d.style.mono == Some(true) {
            font.family = mono_family.clone();
        }
    }
    let run = TextRun {
        len: text.len(),
        font,
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

/// Shapes `text`, truncating with an ellipsis to fit `width`.
fn shape_fit(
    window: &Window,
    text: &str,
    color: Hsla,
    d: Option<&DisplayCell>,
    mono: &SharedString,
    width: Pixels,
) -> ShapedLine {
    let line = shape(window, text, color, d, mono);
    if line.width() <= width {
        return line;
    }
    let ell = shape(window, "…", color, d, mono).width();
    let idx = line.closest_index_for_x(width - ell);
    let mut cut = idx.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    shape(window, &format!("{}…", &text[..cut]), color, d, mono)
}

impl IntoElement for GridElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GridElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepared;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.bounds_out.set(bounds);
        let theme = cx.theme();
        let dark = theme.is_dark();
        let colors = Colors {
            bg: theme.background,
            header: theme.secondary,
            header_active: theme.secondary_active,
            line: theme.border.opacity(0.6),
            text: theme.foreground,
            muted: theme.muted_foreground,
            header_text: theme.muted_foreground,
            select: theme.primary,
            select_fill: theme.primary.opacity(0.12),
            error: theme.danger,
            warning: theme.warning,
            boundary: theme.muted_foreground.opacity(0.55),
        };
        let mono = theme.mono_font_family.clone();
        let ws = self.view.read(cx);
        let origin = bounds.origin;
        let body = Bounds::new(
            point(
                origin.x + px(HEADER_W as f32),
                origin.y + px(HEADER_H as f32),
            ),
            size(
                (bounds.size.width - px(HEADER_W as f32)).max(px(0.)),
                (bounds.size.height - px(HEADER_H as f32)).max(px(0.)),
            ),
        );

        // Code-side flip: scale x about the body's centre; switch content at
        // the midpoint; hide text while animating.
        let progress = ws.flip_progress();
        let animating = progress.is_some();
        let (scale, code) = match progress {
            Some(p) => (
                (1.0 - 2.0 * p).abs(),
                if p < 0.5 { !ws.code_side } else { ws.code_side },
            ),
            None => (1.0, ws.code_side),
        };
        let tinted = ws.peek || code;
        let centre = body.origin.x + body.size.width / 2.0;
        let sx = |x: Pixels| centre + (x - centre) * scale;

        let vis = ws.visible();
        let (scx, scy) = ws.scroll;
        let x_of = |c: u32| body.origin.x + px((ws.cols.offset(c) - scx) as f32);
        let y_of = |r: u32| body.origin.y + px((ws.rows.offset(r) - scy) as f32);
        let cell_bounds = |r: u32, c: u32| {
            let (x0, x1) = (sx(x_of(c)), sx(x_of(c) + px(ws.cols.size(c) as f32)));
            Bounds::new(
                point(x0, y_of(r)),
                size(x1 - x0, px(ws.rows.size(r) as f32)),
            )
        };
        let rect_bounds = |r: Rect| {
            let (r1, r2) = (r.r1.max(vis.r1), r.r2.min(vis.r2.saturating_add(1)));
            let (c1, c2) = (r.c1.max(vis.c1), r.c2.min(vis.c2.saturating_add(1)));
            (r2 >= r1 && c2 >= c1).then(|| {
                let a = cell_bounds(r1, c1);
                let b = cell_bounds(r2, c2);
                Bounds::from_corners(
                    a.origin,
                    point(b.origin.x + b.size.width, b.origin.y + b.size.height),
                )
            })
        };
        let sheet = ws.sheet;
        let get =
            |r: u32, c: u32| -> Option<&DisplayCell> { ws.cache.get(&CellAddr::new(sheet?, r, c)) };
        let has_text = |r: u32, c: u32| get(r, c).is_some_and(|d| !d.text.is_empty());
        let hash_at = |r: u32, c: u32| -> Option<Option<MetaHash>> { get(r, c).map(|d| d.hash) };
        let meta_colors = ws.info.as_ref().map(|i| &i.meta_colors);
        let color_of = |h: MetaHash| -> (Hsla, bool) {
            let idx = meta_colors.and_then(|m| m.get(&h)).copied().unwrap_or(0);
            (palette::color(idx, dark), palette::hatched(idx))
        };

        let mut cells = Vec::new();
        for r in vis.r1..=vis.r2 {
            for c in vis.c1..=vis.c2 {
                let Some(d) = get(r, c) else { continue };
                let b = cell_bounds(r, c);
                let bg = d.style.bg.map(rgb_hsla);
                let tint = d.hash.filter(|_| tinted).map(|h| {
                    let (col, hatch) = color_of(h);
                    (col.opacity(if code { 0.55 } else { 0.4 }), hatch)
                });
                let marker = if d.error.is_some() || d.display_error.is_some() {
                    Some(colors.error)
                } else if d.warning.is_some() {
                    Some(colors.warning)
                } else {
                    None
                };
                let mut clip = b;
                let (text, color, align) = if code {
                    (
                        d.label.as_deref().unwrap_or("").to_string(),
                        colors.muted,
                        Align::Left,
                    )
                } else {
                    (
                        d.text.clone(),
                        d.style.fg.map(rgb_hsla).unwrap_or(colors.text),
                        d.align(),
                    )
                };
                let line = if text.is_empty() || animating {
                    None
                } else {
                    let style_src = (!code).then_some(d);
                    let full = shape(window, &text, color, style_src, &mono);
                    let need = full.width() + px(2.0 * PAD);
                    // Spill into empty neighbours (left-aligned spills right,
                    // right-aligned spills left).
                    if need > b.size.width && !code {
                        match align {
                            Align::Left => {
                                let mut c2 = c;
                                while clip.size.width < need && c2 < vis.c2 && !has_text(r, c2 + 1)
                                {
                                    c2 += 1;
                                    clip.size.width += px(ws.cols.size(c2) as f32);
                                }
                            }
                            Align::Right => {
                                let mut c1 = c;
                                while clip.size.width < need && c1 > vis.c1 && !has_text(r, c1 - 1)
                                {
                                    c1 -= 1;
                                    let w = px(ws.cols.size(c1) as f32);
                                    clip.origin.x -= w;
                                    clip.size.width += w;
                                }
                            }
                            Align::Centre => {}
                        }
                    }
                    let avail = clip.size.width - px(2.0 * PAD);
                    let line = if full.width() > avail {
                        shape_fit(window, &text, color, style_src, &mono, avail)
                    } else {
                        full
                    };
                    let tx = match align {
                        Align::Left => clip.origin.x + px(PAD),
                        Align::Right => clip.origin.x + clip.size.width - px(PAD) - line.width(),
                        Align::Centre => clip.origin.x + (clip.size.width - line.width()) / 2.0,
                    };
                    let ty = b.origin.y + (b.size.height - window.line_height()) / 2.0;
                    Some((line, point(tx, ty)))
                };
                cells.push(PaintCell {
                    bounds: b,
                    bg,
                    tint,
                    clip,
                    line,
                    marker,
                });
            }
        }

        // Automatic boundaries: between adjacent cells whose hashes differ
        // when at least one of them is non-empty.
        let mut boundaries = Vec::new();
        if ws.boundaries || code {
            let width = px(if code { 2.0 } else { 1.5 });
            let pick = |a: Option<Option<MetaHash>>, b: Option<Option<MetaHash>>| -> Hsla {
                if !tinted {
                    return colors.boundary;
                }
                match a.flatten().or(b.flatten()) {
                    Some(h) => {
                        let (c, _) = color_of(h);
                        if dark {
                            c
                        } else {
                            c.opacity(1.0).blend(Hsla::black().opacity(0.25))
                        }
                    }
                    None => colors.boundary,
                }
            };
            for r in vis.r1..=vis.r2 {
                for c in vis.c1..=vis.c2 {
                    let here = hash_at(r, c);
                    let right = hash_at(r, c + 1);
                    let below = hash_at(r + 1, c);
                    let b = cell_bounds(r, c);
                    let differs = |a: Option<Option<MetaHash>>, n: Option<Option<MetaHash>>| {
                        (a.is_some() || n.is_some()) && a.flatten() != n.flatten()
                    };
                    if differs(here, right) {
                        boundaries.push((
                            Bounds::new(
                                point(b.origin.x + b.size.width - width / 2.0, b.origin.y),
                                size(width, b.size.height),
                            ),
                            pick(here, right),
                        ));
                    }
                    if differs(here, below) {
                        boundaries.push((
                            Bounds::new(
                                point(b.origin.x, b.origin.y + b.size.height - width / 2.0),
                                size(b.size.width, width),
                            ),
                            pick(here, below),
                        ));
                    }
                }
            }
            // Left/top edges of the visible area.
            if vis.c1 > 0 {
                for r in vis.r1..=vis.r2 {
                    let (l, here) = (hash_at(r, vis.c1 - 1), hash_at(r, vis.c1));
                    if (l.is_some() || here.is_some()) && l.flatten() != here.flatten() {
                        let b = cell_bounds(r, vis.c1);
                        boundaries.push((
                            Bounds::new(
                                point(b.origin.x - width / 2.0, b.origin.y),
                                size(width, b.size.height),
                            ),
                            pick(here, l),
                        ));
                    }
                }
            }
        }

        let hlines = (vis.r1..=vis.r2.saturating_add(1)).map(y_of).collect();
        let vlines = (vis.c1..=vis.c2.saturating_add(1))
            .map(|c| sx(x_of(c)))
            .collect();

        let sel_rect = ws.sel.rect();
        let col_heads = (vis.c1..=vis.c2)
            .map(|c| {
                let b = Bounds::new(
                    point(x_of(c), origin.y),
                    size(px(ws.cols.size(c) as f32), px(HEADER_H as f32)),
                );
                let active = sel_rect.c1 <= c && c <= sel_rect.c2;
                (
                    b,
                    shape(
                        window,
                        &(u64::from(c) + 1).to_string(),
                        colors.header_text,
                        None,
                        &mono,
                    ),
                    active,
                )
            })
            .collect();
        let row_heads = (vis.r1..=vis.r2)
            .map(|r| {
                let b = Bounds::new(
                    point(origin.x, y_of(r)),
                    size(px(HEADER_W as f32), px(ws.rows.size(r) as f32)),
                );
                let active = sel_rect.r1 <= r && r <= sel_rect.r2;
                (
                    b,
                    shape(
                        window,
                        &(u64::from(r) + 1).to_string(),
                        colors.header_text,
                        None,
                        &mono,
                    ),
                    active,
                )
            })
            .collect();

        let sel = rect_bounds(sel_rect);
        let (cr, cc) = ws.sel.cursor;
        let cursor =
            (ws.sel.mode == SelMode::Cells && vis.contains(cr, cc)).then(|| cell_bounds(cr, cc));
        let handle = (!animating && !code)
            .then(|| ws.fill_handle())
            .flatten()
            .filter(|p| body.contains(p))
            .map(|p| {
                Bounds::new(
                    point(p.x - px(HANDLE / 2.0), p.y - px(HANDLE / 2.0)),
                    size(px(HANDLE), px(HANDLE)),
                )
            });
        let fill_preview = ws.fill_drag.and_then(rect_bounds);
        let autofill = ws
            .autofill
            .as_ref()
            .map(|a| {
                a.cells
                    .iter()
                    .filter(|x| Some(x.sheet) == sheet && vis.contains(x.row, x.col))
                    .map(|x| cell_bounds(x.row, x.col))
                    .collect()
            })
            .unwrap_or_default();
        let (share, share_color) = match &ws.share_hl {
            Some(hl) => (
                hl.cells
                    .iter()
                    .filter(|x| Some(x.sheet) == sheet && vis.contains(x.row, x.col))
                    .map(|x| cell_bounds(x.row, x.col))
                    .collect(),
                color_of(hl.hash).0,
            ),
            None => (Vec::new(), colors.select),
        };

        Prepared {
            colors,
            body,
            cells,
            boundaries,
            hlines,
            vlines,
            col_heads,
            row_heads,
            sel,
            cursor,
            handle,
            fill_preview,
            autofill,
            share,
            share_color,
            animating,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        p: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let c = &p.colors;
        let body = p.body;
        let line_h = window.line_height();
        window.paint_quad(fill(bounds, c.bg));

        window.with_content_mask(Some(ContentMask { bounds: body }), |window| {
            for cell in &p.cells {
                if let Some(bg) = cell.bg {
                    window.paint_quad(fill(cell.bounds, bg));
                }
                if let Some((tint, hatch)) = cell.tint {
                    window.paint_quad(fill(cell.bounds, tint));
                    if hatch {
                        paint_hatch(window, cell.bounds, tint.opacity(1.0));
                    }
                }
            }
            for &y in &p.hlines {
                window.paint_quad(fill(
                    Bounds::new(point(body.origin.x, y), size(body.size.width, px(1.))),
                    c.line,
                ));
            }
            for &x in &p.vlines {
                window.paint_quad(fill(
                    Bounds::new(point(x, body.origin.y), size(px(1.), body.size.height)),
                    c.line,
                ));
            }
            for (b, color) in &p.boundaries {
                window.paint_quad(fill(*b, *color));
            }
            if let Some(s) = p.sel {
                window.paint_quad(fill(s, c.select_fill));
            }
            for cell in &p.cells {
                if let Some((line, at)) = &cell.line {
                    window.with_content_mask(Some(ContentMask { bounds: cell.clip }), |window| {
                        if cell.clip.size.width > cell.bounds.size.width {
                            // Spilled text covers the neighbours' grid lines.
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(cell.clip.origin.x + px(1.), cell.clip.origin.y + px(1.)),
                                    size(
                                        cell.clip.size.width - px(1.),
                                        cell.clip.size.height - px(1.),
                                    ),
                                ),
                                cell.bg.unwrap_or(c.bg),
                            ));
                        }
                        let _ = line.paint(*at, line_h, TextAlign::Left, None, window, cx);
                    });
                }
                if let Some(m) = cell.marker {
                    let b = cell.bounds;
                    let tr = point(b.origin.x + b.size.width, b.origin.y);
                    let mut path = PathBuilder::fill();
                    path.move_to(tr);
                    path.line_to(point(tr.x - px(MARKER), tr.y));
                    path.line_to(point(tr.x, tr.y + px(MARKER)));
                    path.close();
                    if let Ok(path) = path.build() {
                        window.paint_path(path, m);
                    }
                }
            }
            if let Some(s) = p.sel {
                paint_outline(window, s, px(1.), c.select);
            }
            if let Some(cur) = p.cursor {
                paint_outline(window, cur, px(2.), c.select);
            }
            if let Some(h) = p.handle {
                window.paint_quad(fill(
                    Bounds::new(
                        point(h.origin.x - px(1.), h.origin.y - px(1.)),
                        size(h.size.width + px(2.), h.size.height + px(2.)),
                    ),
                    c.bg,
                ));
                window.paint_quad(fill(h, c.select));
            }
            if let Some(f) = p.fill_preview {
                paint_outline(window, f, px(2.), c.select.opacity(0.6));
            }
            for b in &p.autofill {
                paint_outline(window, *b, px(2.), c.warning);
            }
            for b in &p.share {
                paint_outline(
                    window,
                    *b,
                    px(2.),
                    p.share_color.blend(Hsla::black().opacity(0.3)),
                );
            }
        });

        // Frozen headers.
        let col_strip = Bounds::new(
            point(body.origin.x, bounds.origin.y),
            size(body.size.width, px(HEADER_H as f32)),
        );
        let row_strip = Bounds::new(
            point(bounds.origin.x, body.origin.y),
            size(px(HEADER_W as f32), body.size.height),
        );
        window.paint_quad(fill(col_strip, c.header));
        window.paint_quad(fill(row_strip, c.header));
        window.with_content_mask(Some(ContentMask { bounds: col_strip }), |window| {
            for (b, line, active) in &p.col_heads {
                if *active {
                    window.paint_quad(fill(*b, c.header_active));
                }
                window.paint_quad(fill(
                    Bounds::new(point(b.origin.x, b.origin.y), size(px(1.), b.size.height)),
                    c.line,
                ));
                let at = point(
                    b.origin.x + (b.size.width - line.width()) / 2.0,
                    b.origin.y + (b.size.height - line_h) / 2.0,
                );
                let _ = line.paint(at, line_h, TextAlign::Left, None, window, cx);
            }
        });
        window.with_content_mask(Some(ContentMask { bounds: row_strip }), |window| {
            for (b, line, active) in &p.row_heads {
                if *active {
                    window.paint_quad(fill(*b, c.header_active));
                }
                window.paint_quad(fill(
                    Bounds::new(point(b.origin.x, b.origin.y), size(b.size.width, px(1.))),
                    c.line,
                ));
                let at = point(
                    b.origin.x + b.size.width - line.width() - px(PAD),
                    b.origin.y + (b.size.height - line_h) / 2.0,
                );
                let _ = line.paint(at, line_h, TextAlign::Left, None, window, cx);
            }
        });
        window.paint_quad(fill(
            Bounds::new(
                bounds.origin,
                size(px(HEADER_W as f32), px(HEADER_H as f32)),
            ),
            c.header,
        ));
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, body.origin.y - px(1.)),
                size(bounds.size.width, px(1.)),
            ),
            c.line,
        ));
        window.paint_quad(fill(
            Bounds::new(
                point(body.origin.x - px(1.), bounds.origin.y),
                size(px(1.), bounds.size.height),
            ),
            c.line,
        ));

        let measuring = self.view.update(cx, |ws, _| {
            if let Some(f) = ws.fps.as_mut() {
                f.frame();
            }
            ws.fps.is_some()
        });
        if measuring || p.animating {
            window.request_animation_frame();
        }
    }
}

fn paint_outline(window: &mut Window, b: Bounds<Pixels>, w: Pixels, color: Hsla) {
    let (x, y, bw, bh) = (b.origin.x, b.origin.y, b.size.width, b.size.height);
    window.paint_quad(fill(Bounds::new(point(x, y), size(bw, w)), color));
    window.paint_quad(fill(Bounds::new(point(x, y + bh - w), size(bw, w)), color));
    window.paint_quad(fill(Bounds::new(point(x, y), size(w, bh)), color));
    window.paint_quad(fill(Bounds::new(point(x + bw - w, y), size(w, bh)), color));
}

/// Diagonal hatch for colour indexes beyond the 16-hue palette.
fn paint_hatch(window: &mut Window, b: Bounds<Pixels>, color: Hsla) {
    window.with_content_mask(Some(ContentMask { bounds: b }), |window| {
        let step = px(7.);
        let h = b.size.height;
        let mut x = b.origin.x - h;
        while x < b.origin.x + b.size.width {
            let mut path = PathBuilder::stroke(px(1.));
            path.move_to(point(x, b.origin.y + h));
            path.line_to(point(x + h, b.origin.y));
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
            x += step;
        }
    });
}
