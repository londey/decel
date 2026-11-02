// SPDX-License-Identifier: MIT OR Apache-2.0

//! Phase 0 spike: a virtualised dummy grid painted by a custom element.
//!
//! Only the visible cells are shaped and painted. Scrolling moves an offset
//! held by [`GridView`]; the element recomputes the visible range each frame.

use gpui::{
    App, Bounds, ContentMask, Context, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, InteractiveElement as _, IntoElement, LayoutId, ParentElement as _, Pixels,
    Point, Render, ScrollEvent, ShapedLine, SharedString, Style, Styled as _, TextRun, Window, div,
    fill, point, px, relative, size,
};
use gpui_component::ActiveTheme as _;

const ROW_HEIGHT: f32 = 22.;
const COL_WIDTH: f32 = 96.;
const HEADER_HEIGHT: f32 = 24.;
const HEADER_WIDTH: f32 = 56.;

pub struct GridView {
    rows: u32,
    cols: u32,
    offset: Point<Pixels>,
}

impl GridView {
    pub fn new(rows: u32, cols: u32) -> Self {
        Self {
            rows,
            cols,
            offset: Point::default(),
        }
    }

    fn content_size(&self) -> (Pixels, Pixels) {
        (
            px(COL_WIDTH * self.cols as f32),
            px(ROW_HEIGHT * self.rows as f32),
        )
    }

    fn on_scroll(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(ROW_HEIGHT));
        let viewport = window.viewport_size();
        let (w, h) = self.content_size();
        let max_x = (w - viewport.width + px(HEADER_WIDTH)).max(px(0.));
        let max_y = (h - viewport.height + px(HEADER_HEIGHT * 2.)).max(px(0.));
        self.offset.x = (self.offset.x - delta.x).clamp(px(0.), max_x);
        self.offset.y = (self.offset.y - delta.y).clamp(px(0.), max_y);
        cx.notify();
    }
}

impl Render for GridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("spike-grid")
            .size_full()
            .on_scroll(cx.listener(Self::on_scroll))
            .child(GridElement {
                rows: self.rows,
                cols: self.cols,
                offset: self.offset,
                colors: GridColors {
                    background: theme.background,
                    header: theme.secondary,
                    line: theme.border,
                    text: theme.foreground,
                    header_text: theme.muted_foreground,
                },
            })
    }
}

#[derive(Clone, Copy)]
struct GridColors {
    background: Hsla,
    header: Hsla,
    line: Hsla,
    text: Hsla,
    header_text: Hsla,
}

struct GridElement {
    rows: u32,
    cols: u32,
    offset: Point<Pixels>,
    colors: GridColors,
}

/// Visible rows and columns plus their shaped labels, built in prepaint.
struct GridLayout {
    first_row: u32,
    last_row: u32,
    first_col: u32,
    last_col: u32,
    cells: Vec<(Point<Pixels>, ShapedLine)>,
    row_labels: Vec<(Point<Pixels>, ShapedLine)>,
    col_labels: Vec<(Point<Pixels>, ShapedLine)>,
}

/// Spreadsheet-style column name: 0 → A, 25 → Z, 26 → AA.
fn column_name(mut col: u32) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    name.reverse();
    String::from_utf8_lossy(&name).into_owned()
}

fn shape(window: &mut Window, text: String, color: Hsla) -> ShapedLine {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(SharedString::from(text), font_size, &[run], None)
}

impl IntoElement for GridElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GridElement {
    type RequestLayoutState = ();
    type PrepaintState = GridLayout;

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
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let body_w = f32::from(bounds.size.width) - HEADER_WIDTH;
        let body_h = f32::from(bounds.size.height) - HEADER_HEIGHT;
        let off_x = f32::from(self.offset.x);
        let off_y = f32::from(self.offset.y);

        let first_row = (off_y / ROW_HEIGHT) as u32;
        let last_row = (((off_y + body_h) / ROW_HEIGHT).ceil() as u32).min(self.rows);
        let first_col = (off_x / COL_WIDTH) as u32;
        let last_col = (((off_x + body_w) / COL_WIDTH).ceil() as u32).min(self.cols);

        let origin = bounds.origin;
        let cell_origin = |row: u32, col: u32| {
            point(
                origin.x + px(HEADER_WIDTH + col as f32 * COL_WIDTH - off_x + 6.),
                origin.y + px(HEADER_HEIGHT + row as f32 * ROW_HEIGHT - off_y),
            )
        };

        let mut cells = Vec::new();
        for row in first_row..last_row {
            for col in first_col..last_col {
                let text = format!("{}", (row as u64 + 1) * (col as u64 + 1));
                cells.push((cell_origin(row, col), shape(window, text, self.colors.text)));
            }
        }
        let row_labels = (first_row..last_row)
            .map(|row| {
                let at = point(origin.x + px(6.), cell_origin(row, 0).y);
                (
                    at,
                    shape(window, (row + 1).to_string(), self.colors.header_text),
                )
            })
            .collect();
        let col_labels = (first_col..last_col)
            .map(|col| {
                let at = point(cell_origin(0, col).x, origin.y);
                (at, shape(window, column_name(col), self.colors.header_text))
            })
            .collect();

        GridLayout {
            first_row,
            last_row,
            first_col,
            last_col,
            cells,
            row_labels,
            col_labels,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let colors = self.colors;
        let origin = bounds.origin;
        let line_h = px(ROW_HEIGHT);
        let off = self.offset;
        let body = Bounds::new(
            point(origin.x + px(HEADER_WIDTH), origin.y + px(HEADER_HEIGHT)),
            size(
                bounds.size.width - px(HEADER_WIDTH),
                bounds.size.height - px(HEADER_HEIGHT),
            ),
        );

        window.paint_quad(fill(bounds, colors.background));

        // Body: gridlines and cell text, clipped to the body area.
        window.with_content_mask(Some(ContentMask { bounds: body }), |window| {
            for row in layout.first_row..=layout.last_row {
                let y = body.origin.y + px(row as f32 * ROW_HEIGHT) - off.y;
                window.paint_quad(fill(
                    Bounds::new(point(body.origin.x, y), size(body.size.width, px(1.))),
                    colors.line,
                ));
            }
            for col in layout.first_col..=layout.last_col {
                let x = body.origin.x + px(col as f32 * COL_WIDTH) - off.x;
                window.paint_quad(fill(
                    Bounds::new(point(x, body.origin.y), size(px(1.), body.size.height)),
                    colors.line,
                ));
            }
            for (at, line) in &layout.cells {
                let _ = line.paint(*at, line_h, gpui::TextAlign::Left, None, window, cx);
            }
        });

        // Column header strip.
        let col_header = Bounds::new(
            point(origin.x + px(HEADER_WIDTH), origin.y),
            size(bounds.size.width - px(HEADER_WIDTH), px(HEADER_HEIGHT)),
        );
        window.paint_quad(fill(col_header, colors.header));
        window.with_content_mask(Some(ContentMask { bounds: col_header }), |window| {
            for (at, line) in &layout.col_labels {
                let _ = line.paint(
                    *at,
                    px(HEADER_HEIGHT),
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });

        // Row header strip.
        let row_header = Bounds::new(
            point(origin.x, origin.y + px(HEADER_HEIGHT)),
            size(px(HEADER_WIDTH), bounds.size.height - px(HEADER_HEIGHT)),
        );
        window.paint_quad(fill(row_header, colors.header));
        window.with_content_mask(Some(ContentMask { bounds: row_header }), |window| {
            for (at, line) in &layout.row_labels {
                let _ = line.paint(*at, line_h, gpui::TextAlign::Left, None, window, cx);
            }
        });

        // Corner.
        window.paint_quad(fill(
            Bounds::new(origin, size(px(HEADER_WIDTH), px(HEADER_HEIGHT))),
            colors.header,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::column_name;

    #[test]
    fn column_names() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(51), "AZ");
        assert_eq!(column_name(52), "BA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }
}
