// SPDX-License-Identifier: MIT OR Apache-2.0

//! The virtualised grid element (DESIGN §8.2).
//!
//! Only visible cells are shaped and painted. Painting order: backgrounds,
//! grid lines, selection fill, text, markers, selection outline, then the
//! frozen headers on top.

use std::cell::Cell as StdCell;
use std::rc::Rc;

use decel_engine::{Align, DisplayCell};
use decel_model::CellAddr;
use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Entity, FontStyle, FontWeight, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, PathBuilder, Pixels, Point, ShapedLine,
    SharedString, Style, TextAlign, TextRun, Window, fill, point, px, relative, rgb, size,
};
use gpui_component::ActiveTheme as _;

use crate::workspace::{HEADER_H, HEADER_W, SelMode, Workspace};

const PAD: f32 = 5.0;
const MARKER: f32 = 6.0;

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
    header_text: Hsla,
    select: Hsla,
    select_fill: Hsla,
    error: Hsla,
    warning: Hsla,
}

struct PaintCell {
    bounds: Bounds<Pixels>,
    bg: Option<Hsla>,
    /// Text clip (spilled into empty neighbours when the text overflows).
    clip: Bounds<Pixels>,
    line: Option<(ShapedLine, Point<Pixels>)>,
    marker: Option<Hsla>,
}

pub struct Prepared {
    colors: Colors,
    body: Bounds<Pixels>,
    cells: Vec<PaintCell>,
    hlines: Vec<Pixels>,
    vlines: Vec<Pixels>,
    col_heads: Vec<(Bounds<Pixels>, ShapedLine, bool)>,
    row_heads: Vec<(Bounds<Pixels>, ShapedLine, bool)>,
    sel: Option<Bounds<Pixels>>,
    cursor: Option<Bounds<Pixels>>,
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
        let colors = Colors {
            bg: theme.background,
            header: theme.secondary,
            header_active: theme.secondary_active,
            line: theme.border.opacity(0.6),
            text: theme.foreground,
            header_text: theme.muted_foreground,
            select: theme.primary,
            select_fill: theme.primary.opacity(0.12),
            error: theme.danger,
            warning: theme.warning,
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
        let vis = ws.visible();
        let (sx, sy) = ws.scroll;
        let x_of = |c: u32| body.origin.x + px((ws.cols.offset(c) - sx) as f32);
        let y_of = |r: u32| body.origin.y + px((ws.rows.offset(r) - sy) as f32);
        let cell_bounds = |r: u32, c: u32| {
            Bounds::new(
                point(x_of(c), y_of(r)),
                size(px(ws.cols.size(c) as f32), px(ws.rows.size(r) as f32)),
            )
        };
        let sheet = ws.sheet;
        let get =
            |r: u32, c: u32| -> Option<&DisplayCell> { ws.cache.get(&CellAddr::new(sheet?, r, c)) };
        let has_text = |r: u32, c: u32| get(r, c).is_some_and(|d| !d.text.is_empty());

        let mut cells = Vec::new();
        for r in vis.r1..=vis.r2 {
            for c in vis.c1..=vis.c2 {
                let Some(d) = get(r, c) else { continue };
                let b = cell_bounds(r, c);
                let bg = d.style.bg.map(rgb_hsla);
                let marker = if d.error.is_some() || d.display_error.is_some() {
                    Some(colors.error)
                } else if d.warning.is_some() {
                    Some(colors.warning)
                } else {
                    None
                };
                let mut clip = b;
                let line = if d.text.is_empty() {
                    None
                } else {
                    let color = d.style.fg.map(rgb_hsla).unwrap_or(colors.text);
                    let full = shape(window, &d.text, color, Some(d), &mono);
                    let need = full.width() + px(2.0 * PAD);
                    let align = d.align();
                    // Spill into empty neighbours (left-aligned spills right,
                    // right-aligned spills left).
                    if need > b.size.width {
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
                        shape_fit(window, &d.text, color, Some(d), &mono, avail)
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
                    clip,
                    line,
                    marker,
                });
            }
        }

        let hlines = (vis.r1..=vis.r2.saturating_add(1)).map(y_of).collect();
        let vlines = (vis.c1..=vis.c2.saturating_add(1)).map(x_of).collect();

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

        // Selection, clipped to the visible area.
        let clamp_r = |r: u32| r.clamp(vis.r1, vis.r2.saturating_add(1));
        let clamp_c = |c: u32| c.clamp(vis.c1, vis.c2.saturating_add(1));
        let sel = {
            let (r1, r2) = (clamp_r(sel_rect.r1), clamp_r(sel_rect.r2.saturating_add(1)));
            let (c1, c2) = (clamp_c(sel_rect.c1), clamp_c(sel_rect.c2.saturating_add(1)));
            (r2 > r1 && c2 > c1)
                .then(|| Bounds::from_corners(point(x_of(c1), y_of(r1)), point(x_of(c2), y_of(r2))))
        };
        let (cr, cc) = ws.sel.cursor;
        let cursor =
            (ws.sel.mode == SelMode::Cells && vis.contains(cr, cc)).then(|| cell_bounds(cr, cc));

        Prepared {
            colors,
            body,
            cells,
            hlines,
            vlines,
            col_heads,
            row_heads,
            sel,
            cursor,
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
            if let Some(s) = p.sel {
                window.paint_quad(fill(s, c.select_fill));
            }
            for cell in &p.cells {
                if let Some((line, at)) = &cell.line {
                    // Spilled text must not erase neighbours' grid lines, but
                    // it covers them, as in other spreadsheets.
                    window.with_content_mask(Some(ContentMask { bounds: cell.clip }), |window| {
                        if cell.clip.size.width > cell.bounds.size.width {
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

        self.view.update(cx, |ws, _| {
            if let Some(f) = ws.fps.as_mut() {
                f.frame();
            }
        });
        if self.view.read(cx).fps.is_some() {
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
