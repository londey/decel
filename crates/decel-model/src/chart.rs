// SPDX-License-Identifier: MIT OR Apache-2.0

//! Chart specifications (DESIGN §8.6), stored as JSON in `ChartRecord::spec`.

use serde::{Deserialize, Serialize};

use crate::{Axis, ChartRecord, Rect, SheetId, StructOp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    #[default]
    Line,
    Bar,
    Scatter,
}

/// A rectangle of cells on a sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeRef {
    pub sheet: SheetId,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub name: String,
    pub range: RangeRef,
    /// `#rrggbb`, or `None` for the automatic palette colour.
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSpec {
    #[serde(default)]
    pub kind: ChartKind,
    #[serde(default)]
    pub title: String,
    /// Categories (line/bar) or x values (scatter). `None`: 1, 2, 3, …
    #[serde(default)]
    pub x: Option<RangeRef>,
    #[serde(default)]
    pub series: Vec<Series>,
    #[serde(default)]
    pub x_title: String,
    #[serde(default)]
    pub y_title: String,
    #[serde(default = "yes")]
    pub legend: bool,
    #[serde(default = "default_w")]
    pub width: f32,
    #[serde(default = "default_h")]
    pub height: f32,
}

fn yes() -> bool {
    true
}

fn default_w() -> f32 {
    480.0
}

fn default_h() -> f32 {
    300.0
}

impl Default for ChartSpec {
    fn default() -> Self {
        Self {
            kind: ChartKind::Line,
            title: String::new(),
            x: None,
            series: Vec::new(),
            x_title: String::new(),
            y_title: String::new(),
            legend: true,
            width: default_w(),
            height: default_h(),
        }
    }
}

impl ChartSpec {
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Every range the chart reads.
    pub fn ranges(&self) -> impl Iterator<Item = RangeRef> + '_ {
        self.x
            .iter()
            .copied()
            .chain(self.series.iter().map(|s| s.range))
    }
}

/// Maps an inclusive span `[lo, hi]` through a structural op on its axis.
/// A span entirely inside a deleted block collapses to the row/column now
/// at that position.
fn map_span(op: StructOp, lo: u32, hi: u32) -> (u32, u32) {
    let (at, n) = op.span();
    if op.is_delete() {
        let end = at.saturating_add(n);
        let map_lo = |i: u32| {
            if i < at {
                i
            } else if i < end {
                at
            } else {
                i - n
            }
        };
        let map_hi = |i: u32| {
            if i < at {
                i
            } else if i < end {
                at.saturating_sub(1)
            } else {
                i - n
            }
        };
        let (l, h) = (map_lo(lo), map_hi(hi));
        if h < l { (l, l) } else { (l, h) }
    } else if at <= lo {
        (lo.saturating_add(n), hi.saturating_add(n))
    } else if at <= hi {
        (lo, hi.saturating_add(n))
    } else {
        (lo, hi)
    }
}

fn shift_range(r: RangeRef, op: StructOp) -> RangeRef {
    if r.sheet != op.sheet() {
        return r;
    }
    let mut rect = r.rect;
    match op.axis() {
        Axis::Row => (rect.r1, rect.r2) = map_span(op, rect.r1, rect.r2),
        Axis::Col => (rect.c1, rect.c2) = map_span(op, rect.c1, rect.c2),
    }
    RangeRef { rect, ..r }
}

impl ChartRecord {
    /// The record after a structural op: the anchor and every source range on
    /// the op's sheet move with the cells. Specs that don't parse keep their
    /// text and only the anchor moves.
    pub fn shifted(&self, op: StructOp) -> ChartRecord {
        let mut out = self.clone();
        if self.sheet == op.sheet() {
            let idx = match op.axis() {
                Axis::Row => &mut out.row,
                Axis::Col => &mut out.col,
            };
            *idx = map_span(op, *idx, *idx).0;
        }
        if let Some(mut spec) = ChartSpec::parse(&self.spec) {
            spec.x = spec.x.map(|r| shift_range(r, op));
            for s in &mut spec.series {
                s.range = shift_range(s.range, op);
            }
            out.spec = spec.to_json();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: SheetId = SheetId(1);

    fn rec(row: u32, rect: Rect) -> ChartRecord {
        let spec = ChartSpec {
            series: vec![Series {
                name: "y".into(),
                range: RangeRef { sheet: S, rect },
                color: None,
            }],
            ..ChartSpec::default()
        };
        ChartRecord {
            sheet: S,
            row,
            col: 3,
            spec: spec.to_json(),
        }
    }

    fn range(r: &ChartRecord) -> Rect {
        ChartSpec::parse(&r.spec).unwrap().series[0].range.rect
    }

    #[test]
    fn insert_above_moves_anchor_and_range() {
        let r = rec(10, Rect::new(5, 0, 20, 0));
        let s = r.shifted(StructOp::InsertRows {
            sheet: S,
            at: 2,
            n: 3,
        });
        assert_eq!(s.row, 13);
        assert_eq!(range(&s), Rect::new(8, 0, 23, 0));
        // Inserting inside the range grows it.
        let s = r.shifted(StructOp::InsertRows {
            sheet: S,
            at: 6,
            n: 2,
        });
        assert_eq!(range(&s), Rect::new(5, 0, 22, 0));
        assert_eq!(s.row, 12);
    }

    #[test]
    fn delete_shrinks_and_collapses() {
        let r = rec(10, Rect::new(5, 0, 20, 0));
        let s = r.shifted(StructOp::DeleteRows {
            sheet: S,
            at: 6,
            n: 2,
        });
        assert_eq!(range(&s), Rect::new(5, 0, 18, 0));
        assert_eq!(s.row, 8);
        let s = r.shifted(StructOp::DeleteRows {
            sheet: S,
            at: 0,
            n: 30,
        });
        assert_eq!(range(&s), Rect::new(0, 0, 0, 0));
        assert_eq!(s.row, 0);
        // Columns left of the chart.
        let s = r.shifted(StructOp::InsertCols {
            sheet: S,
            at: 0,
            n: 1,
        });
        assert_eq!(s.col, 4);
        assert_eq!(range(&s), Rect::new(5, 1, 20, 1));
    }

    #[test]
    fn other_sheets_are_untouched() {
        let r = rec(10, Rect::new(5, 0, 20, 0));
        let s = r.shifted(StructOp::InsertRows {
            sheet: SheetId(2),
            at: 0,
            n: 3,
        });
        assert_eq!(s, r);
    }

    #[test]
    fn spec_json_defaults() {
        let s = ChartSpec::parse("{}").unwrap();
        assert!(s.legend);
        assert_eq!(s.kind, ChartKind::Line);
        assert_eq!(ChartSpec::parse(&s.to_json()).unwrap(), s);
    }
}
