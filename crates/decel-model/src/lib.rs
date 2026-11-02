// SPDX-License-Identifier: MIT OR Apache-2.0

//! Pure data types for decel: cell addresses, values, metatable sources and sheets.
//!
//! Rows and columns are **0-based** everywhere in Rust. Lua and the UI are
//! 1-based; convert only at those boundaries.

mod addr;
mod chart;
mod meta;
mod ops;
mod parse;
mod sheet;
mod value;
mod workbook;

pub use addr::{Axis, CellAddr, ChartId, Rect, SheetId};
pub use chart::{ChartKind, ChartSpec, RangeRef, Series};
pub use meta::{
    FIELDS, HOOKS, Member, MemberDiff, MemberKind, MetaHash, MetaParseError, MetaSource,
    diff_members, normalise_source,
};
pub use ops::{Change, ChartRecord, Op, SheetRecord, StructOp, TxnInput, invert_changes};
pub use parse::parse_default;
pub use sheet::{CellStorage, SparseCells};
pub use value::{Cell, CellError, ErrorKind, Value};
pub use workbook::{Layout, Sheet, Workbook};
