// SPDX-License-Identifier: MIT OR Apache-2.0

//! The Lua-facing `c` API (DESIGN §4.3): the cell context, cell handles,
//! range handles and sheet handles.
//!
//! Handles are plain coordinates. Every read goes through the [`Core`] lent
//! to the VM, which records the read as a dependency of the evaluating cell.
//! Rows and columns are 1-based on the Lua side.

use std::sync::Arc;

use decel_model::{CellAddr, CellError, CellStorage, ErrorKind, Rect, SheetId, Value};
use mlua::{
    Function, Lua, MetaMethod, MultiValue, Table, UserData, UserDataFields, UserDataMethods,
    Value as LuaValue,
};

use crate::convert::to_lua;
use crate::core::{Core, Read};
use crate::sandbox::{Interrupt, Propagated};

/// Runs `f` with the lent evaluation state.
fn with_core<R>(lua: &Lua, f: impl FnOnce(&mut Core) -> mlua::Result<R>) -> mlua::Result<R> {
    let mut core = lua
        .try_app_data_mut::<Core>()
        .map_err(|_| mlua::Error::runtime("decel: evaluation state is busy"))?
        .ok_or_else(|| mlua::Error::runtime("decel: no evaluation in progress"))?;
    f(&mut core)
}

fn need(a: CellAddr) -> mlua::Error {
    mlua::Error::external(Interrupt::NeedDep(a))
}

fn propagate(e: CellError) -> mlua::Error {
    mlua::Error::external(Propagated(e))
}

/// Reads a final value, interrupting if it must be evaluated first.
fn read(core: &Core, a: CellAddr) -> mlua::Result<Value> {
    match core.read(a) {
        Read::Ready(v) => Ok(v),
        Read::NeedDep(d) => Err(need(d)),
    }
}

/// Converts a 1-based Lua index to 0-based, or `None` if off the sheet.
fn idx0(v: i64) -> Option<u32> {
    u32::try_from(v - 1).ok()
}

/// A cell position that may be off the sheet (row or column < 1).
#[derive(Debug, Clone, Copy)]
pub struct CellRef {
    pub sheet: SheetId,
    /// 0-based; may be negative.
    pub row: i64,
    pub col: i64,
}

impl CellRef {
    pub fn of(a: CellAddr) -> Self {
        Self {
            sheet: a.sheet,
            row: i64::from(a.row),
            col: i64::from(a.col),
        }
    }

    fn addr(self) -> Option<CellAddr> {
        Some(CellAddr::new(
            self.sheet,
            u32::try_from(self.row).ok()?,
            u32::try_from(self.col).ok()?,
        ))
    }

    fn offset(self, dr: i64, dc: i64) -> Self {
        Self {
            row: self.row.saturating_add(dr),
            col: self.col.saturating_add(dc),
            ..self
        }
    }
}

/// `c`: the evaluating cell's context.
pub struct Ctx(pub CellAddr);

/// A handle to any cell.
pub struct CellHandle(pub CellRef);

/// A rectangle of cells. `rect: None` is an empty range.
#[derive(Clone)]
pub struct RangeHandle {
    pub sheet: SheetId,
    pub rect: Option<Rect>,
    pub exclude: Option<CellAddr>,
    pub kind: Option<Arc<str>>,
}

pub struct SheetHandle(pub SheetId);

fn cell_value(lua: &Lua, r: CellRef) -> mlua::Result<LuaValue> {
    let Some(a) = r.addr() else {
        return Ok(LuaValue::Nil);
    };
    let v = with_core(lua, |core| {
        core.record_cell(a);
        read(core, a)
    })?;
    match v {
        Value::Error(e) => Err(propagate(e)),
        v => to_lua(lua, &v),
    }
}

fn cell_raw(lua: &Lua, r: CellRef) -> mlua::Result<LuaValue> {
    let Some(a) = r.addr() else {
        return Ok(LuaValue::Nil);
    };
    let v = with_core(lua, |core| {
        core.record_cell(a);
        Ok(core
            .cell(a)
            .and_then(|c| c.raw.clone())
            .unwrap_or(Value::Nil))
    })?;
    to_lua(lua, &v)
}

fn cell_error(lua: &Lua, r: CellRef) -> mlua::Result<Option<CellError>> {
    let Some(a) = r.addr() else {
        return Ok(None);
    };
    with_core(lua, |core| {
        core.record_cell(a);
        Ok(match read(core, a)? {
            Value::Error(e) => Some(e),
            _ => None,
        })
    })
}

fn cell_label(lua: &Lua, r: CellRef, unit: bool) -> mlua::Result<Option<String>> {
    let Some(a) = r.addr() else {
        return Ok(None);
    };
    with_core(lua, |core| {
        core.record_cell(a);
        Ok(core
            .meta_info(a)
            .and_then(|i| if unit { i.unit.clone() } else { i.kind.clone() })
            .map(|s| s.to_string()))
    })
}

impl UserData for CellHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("value", |lua, h| cell_value(lua, h.0));
        fields.add_field_method_get("raw", |lua, h| cell_raw(lua, h.0));
        fields.add_field_method_get("ok", |lua, h| Ok(cell_error(lua, h.0)?.is_none()));
        fields.add_field_method_get("err", |lua, h| {
            Ok(cell_error(lua, h.0)?.map(|e| e.message.to_string()))
        });
        fields.add_field_method_get("kind", |lua, h| cell_label(lua, h.0, false));
        fields.add_field_method_get("unit", |lua, h| cell_label(lua, h.0, true));
        fields.add_field_method_get("row", |_, h| Ok(h.0.row + 1));
        fields.add_field_method_get("col", |_, h| Ok(h.0.col + 1));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("offset", |_, h, (dr, dc): (i64, i64)| {
            Ok(CellHandle(h.0.offset(dr, dc)))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, h, ()| {
            Ok(format!("cell({},{})", h.0.row + 1, h.0.col + 1))
        });
    }
}

impl RangeHandle {
    fn new(sheet: SheetId, rect: Option<Rect>, exclude: Option<CellAddr>) -> Self {
        Self {
            sheet,
            rect,
            exclude,
            kind: None,
        }
    }

    /// Records the range dependency and returns its populated cells with
    /// their final values (errors included).
    fn collect(&self, lua: &Lua) -> mlua::Result<Vec<(CellAddr, Value)>> {
        let Some(rect) = self.rect else {
            return Ok(Vec::new());
        };
        with_core(lua, |core| {
            if core.frame.record {
                core.frame.deps.range(self.sheet, rect);
            }
            let Some(sheet) = core.wb.sheet(self.sheet) else {
                return Ok(Vec::new());
            };
            let addrs: Vec<CellAddr> = sheet
                .cells
                .iter_rect(rect)
                .map(|((r, c), _)| CellAddr::new(self.sheet, r, c))
                .filter(|a| Some(*a) != self.exclude)
                .collect();
            let mut out = Vec::with_capacity(addrs.len());
            for a in addrs {
                if let Some(k) = &self.kind
                    && core.meta_info(a).and_then(|i| i.kind.as_deref()) != Some(&**k)
                {
                    continue;
                }
                out.push((a, read(core, a)?));
            }
            Ok(out)
        })
    }

    /// Non-nil values; an error value is re-raised.
    fn values(&self, lua: &Lua) -> mlua::Result<Vec<Value>> {
        let mut out = Vec::new();
        for (_, v) in self.collect(lua)? {
            match v {
                Value::Nil => {}
                Value::Error(e) => return Err(propagate(e)),
                v => out.push(v),
            }
        }
        Ok(out)
    }

    fn numbers(&self, lua: &Lua) -> mlua::Result<Vec<f64>> {
        Ok(self.values(lua)?.iter().filter_map(Value::as_f64).collect())
    }

    /// `(rows, cols)` of the range, with open ends clipped to the last
    /// populated row/column inside it.
    fn dims(&self, lua: &Lua) -> mlua::Result<(u32, u32)> {
        let Some(rect) = self.rect else {
            return Ok((0, 0));
        };
        let cells = self.collect(lua)?;
        let r2 = if rect.r2 == u32::MAX {
            cells.iter().map(|(a, _)| a.row).max().unwrap_or(rect.r1)
        } else {
            rect.r2
        };
        let c2 = if rect.c2 == u32::MAX {
            cells.iter().map(|(a, _)| a.col).max().unwrap_or(rect.c1)
        } else {
            rect.c2
        };
        if cells.is_empty() && (rect.r2 == u32::MAX || rect.c2 == u32::MAX) {
            return Ok((0, 0));
        }
        Ok((r2 - rect.r1 + 1, c2 - rect.c1 + 1))
    }
}

/// Sums numbers in an `i64` while every term is an integer, else in `f64`.
fn sum_values(values: &[Value]) -> Value {
    let mut int: Option<i64> = Some(0);
    let mut float = 0.0;
    for v in values {
        match v {
            Value::Int(i) => {
                int = int.and_then(|s| s.checked_add(*i));
                float += *i as f64;
            }
            Value::Num(n) => {
                int = None;
                float += n;
            }
            _ => {}
        }
    }
    match int {
        Some(i) => Value::Int(i),
        None => Value::Num(float),
    }
}

impl UserData for RangeHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("rows", |lua, r| Ok(r.dims(lua)?.0));
        fields.add_field_method_get("cols", |lua, r| Ok(r.dims(lua)?.1));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("values", |lua, r, ()| {
            let t = lua.create_table()?;
            for v in r.values(lua)? {
                t.raw_push(to_lua(lua, &v)?)?;
            }
            Ok(t)
        });
        methods.add_method("cells", |lua, r, ()| {
            let handles: Vec<CellHandle> = r
                .collect(lua)?
                .into_iter()
                .map(|(a, _)| CellHandle(CellRef::of(a)))
                .collect();
            let mut iter = handles.into_iter();
            lua.create_function_mut(move |_, ()| Ok(iter.next()))
        });
        methods.add_method("sum", |lua, r, ()| {
            to_lua(lua, &sum_values(&r.values(lua)?))
        });
        methods.add_method("count", |lua, r, ()| Ok(r.values(lua)?.len()));
        methods.add_method("avg", |lua, r, ()| {
            let n = r.numbers(lua)?;
            Ok(if n.is_empty() {
                None
            } else {
                Some(n.iter().sum::<f64>() / n.len() as f64)
            })
        });
        methods.add_method("min", |lua, r, ()| {
            Ok(r.numbers(lua)?.into_iter().reduce(f64::min))
        });
        methods.add_method("max", |lua, r, ()| {
            Ok(r.numbers(lua)?.into_iter().reduce(f64::max))
        });
        methods.add_method("map", |lua, r, f: Function| {
            let t = lua.create_table()?;
            for (a, _) in r.collect(lua)? {
                t.raw_push(f.call::<LuaValue>(CellHandle(CellRef::of(a)))?)?;
            }
            Ok(t)
        });
        methods.add_method("filter", |lua, r, f: Function| {
            let t = lua.create_table()?;
            for (a, _) in r.collect(lua)? {
                let h = CellHandle(CellRef::of(a));
                if f.call::<bool>(CellHandle(CellRef::of(a)))? {
                    t.raw_push(h)?;
                }
            }
            Ok(t)
        });
        methods.add_method("reduce", |lua, r, (f, init): (Function, LuaValue)| {
            let mut acc = init;
            for v in r.values(lua)? {
                acc = f.call::<LuaValue>((acc, to_lua(lua, &v)?))?;
            }
            Ok(acc)
        });
        methods.add_method("where_kind", |_, r, kind: String| {
            let mut out = r.clone();
            out.kind = Some(kind.into());
            Ok(out)
        });
        methods.add_meta_method(MetaMethod::ToString, |_, r, ()| {
            Ok(match r.rect {
                Some(rect) => format!(
                    "range({},{}..{},{})",
                    u64::from(rect.r1) + 1,
                    u64::from(rect.c1) + 1,
                    u64::from(rect.r2) + 1,
                    u64::from(rect.c2) + 1
                ),
                None => "range(empty)".into(),
            })
        });
    }
}

/// Rows `r1..=r2` / cols `c1..=c2` from optional 1-based bounds.
fn bounds(lo: Option<i64>, hi: Option<i64>) -> Option<(u32, u32)> {
    let lo = lo.map_or(Some(0), idx0)?;
    let hi = match hi {
        None => u32::MAX,
        Some(h) => idx0(h)?,
    };
    (lo <= hi).then_some((lo, hi))
}

impl UserData for Ctx {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("row", |_, c| Ok(i64::from(c.0.row) + 1));
        fields.add_field_method_get("col", |_, c| Ok(i64::from(c.0.col) + 1));
        fields.add_field_method_get("sheet", |_, c| Ok(SheetHandle(c.0.sheet)));
        fields.add_field_method_get("raw", |lua, c| {
            let v = with_core(lua, |core| {
                Ok(core
                    .cell(c.0)
                    .and_then(|cell| cell.raw.clone())
                    .unwrap_or(Value::Nil))
            })?;
            to_lua(lua, &v)
        });
        fields.add_field_method_get("kind", |lua, c| {
            with_core(lua, |core| {
                Ok(core
                    .meta_info(c.0)
                    .and_then(|i| i.kind.clone())
                    .map(|s| s.to_string()))
            })
        });
        fields.add_field_method_get("unit", |lua, c| {
            with_core(lua, |core| {
                Ok(core
                    .meta_info(c.0)
                    .and_then(|i| i.unit.clone())
                    .map(|s| s.to_string()))
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        let here = |c: &Ctx| CellRef::of(c.0);
        methods.add_method("offset", move |_, c, (dr, dc): (i64, i64)| {
            Ok(CellHandle(here(c).offset(dr, dc)))
        });
        methods.add_method("at", |_, c, (row, col): (i64, i64)| {
            Ok(CellHandle(CellRef {
                sheet: c.0.sheet,
                row: row - 1,
                col: col - 1,
            }))
        });
        methods.add_method("up", move |_, c, n: Option<i64>| {
            Ok(CellHandle(here(c).offset(-n.unwrap_or(1), 0)))
        });
        methods.add_method("down", move |_, c, n: Option<i64>| {
            Ok(CellHandle(here(c).offset(n.unwrap_or(1), 0)))
        });
        methods.add_method("left", move |_, c, n: Option<i64>| {
            Ok(CellHandle(here(c).offset(0, -n.unwrap_or(1))))
        });
        methods.add_method("right", move |_, c, n: Option<i64>| {
            Ok(CellHandle(here(c).offset(0, n.unwrap_or(1))))
        });
        methods.add_method("above", |_, c, ()| {
            let a = c.0;
            let rect = (a.row > 0).then(|| Rect::new(0, a.col, a.row - 1, a.col));
            Ok(RangeHandle::new(a.sheet, rect, None))
        });
        methods.add_method("below", |_, c, ()| {
            let a = c.0;
            let rect = (a.row < u32::MAX).then(|| Rect::new(a.row + 1, a.col, u32::MAX, a.col));
            Ok(RangeHandle::new(a.sheet, rect, None))
        });
        methods.add_method("left_of", |_, c, ()| {
            let a = c.0;
            let rect = (a.col > 0).then(|| Rect::new(a.row, 0, a.row, a.col - 1));
            Ok(RangeHandle::new(a.sheet, rect, None))
        });
        methods.add_method("right_of", |_, c, ()| {
            let a = c.0;
            let rect = (a.col < u32::MAX).then(|| Rect::new(a.row, a.col + 1, a.row, u32::MAX));
            Ok(RangeHandle::new(a.sheet, rect, None))
        });
        methods.add_method("column", |_, c, (r1, r2): (Option<i64>, Option<i64>)| {
            let a = c.0;
            let rect = bounds(r1, r2).map(|(lo, hi)| Rect::new(lo, a.col, hi, a.col));
            Ok(RangeHandle::new(a.sheet, rect, Some(a)))
        });
        methods.add_method("row_range", |_, c, (c1, c2): (Option<i64>, Option<i64>)| {
            let a = c.0;
            let rect = bounds(c1, c2).map(|(lo, hi)| Rect::new(a.row, lo, a.row, hi));
            Ok(RangeHandle::new(a.sheet, rect, Some(a)))
        });
        methods.add_method(
            "rect",
            |_, c, (dr1, dc1, dr2, dc2): (i64, i64, i64, i64)| {
                let a = CellRef::of(c.0);
                let clamp = |v: i64| u32::try_from(v.max(0)).unwrap_or(u32::MAX);
                let (r1, r2) = (a.row + dr1.min(dr2), a.row + dr1.max(dr2));
                let (c1, c2) = (a.col + dc1.min(dc2), a.col + dc1.max(dc2));
                let rect = (r2 >= 0 && c2 >= 0)
                    .then(|| Rect::new(clamp(r1), clamp(c1), clamp(r2), clamp(c2)));
                Ok(RangeHandle::new(c.0.sheet, rect, None))
            },
        );
        methods.add_method("sheet_named", |lua, _c, name: String| {
            let id = with_core(lua, |core| {
                if core.frame.record {
                    core.frame.deps.name(&name);
                }
                Ok(core.wb.sheet_by_name(&name).map(|s| s.id))
            })?;
            match id {
                Some(id) => Ok(SheetHandle(id)),
                None => Err(propagate(CellError::new(
                    ErrorKind::Ref,
                    format!("no sheet named {name:?}"),
                ))),
            }
        });
        methods.add_meta_method(MetaMethod::ToString, |_, c, ()| {
            Ok(format!(
                "c({},{})",
                u64::from(c.0.row) + 1,
                u64::from(c.0.col) + 1
            ))
        });
    }
}

impl UserData for SheetHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |lua, s| {
            with_core(lua, |core| {
                let name = core.wb.sheet(s.0).map(|s| s.name.clone());
                if let Some(n) = &name
                    && core.frame.record
                {
                    core.frame.deps.name(n);
                }
                Ok(name)
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("at", |_, s, (row, col): (i64, i64)| {
            Ok(CellHandle(CellRef {
                sheet: s.0,
                row: row - 1,
                col: col - 1,
            }))
        });
        methods.add_method("range", |_, s, (r1, c1, r2, c2): (i64, i64, i64, i64)| {
            let rect = match (idx0(r1.min(r2)), idx0(c1.min(c2))) {
                (Some(_), Some(_)) => Some(Rect::new(
                    idx0(r1).unwrap_or(0),
                    idx0(c1).unwrap_or(0),
                    idx0(r2).unwrap_or(0),
                    idx0(c2).unwrap_or(0),
                )),
                _ => None,
            };
            Ok(RangeHandle::new(s.0, rect, None))
        });
        methods.add_method("last_row", |lua, s, ()| {
            with_core(lua, |core| {
                if core.frame.record {
                    core.frame
                        .deps
                        .range(s.0, Rect::new(0, 0, u32::MAX, u32::MAX));
                }
                Ok(core
                    .wb
                    .sheet(s.0)
                    .and_then(|sh| sh.cells.last_row())
                    .map_or(0, |r| i64::from(r) + 1))
            })
        });
        methods.add_method("last_col", |lua, s, ()| {
            with_core(lua, |core| {
                if core.frame.record {
                    core.frame
                        .deps
                        .range(s.0, Rect::new(0, 0, u32::MAX, u32::MAX));
                }
                Ok(core
                    .wb
                    .sheet(s.0)
                    .and_then(|sh| sh.cells.last_col())
                    .map_or(0, |c| i64::from(c) + 1))
            })
        });
    }
}

/// The `range` stdlib module: variadic helpers over numbers and ranges.
pub fn range_module(lua: &Lua) -> mlua::Result<Table> {
    fn gather(lua: &Lua, args: MultiValue) -> mlua::Result<Vec<Value>> {
        let mut out = Vec::new();
        for a in args {
            match a {
                LuaValue::Integer(i) => out.push(Value::Int(i)),
                LuaValue::Number(n) => out.push(Value::Num(n)),
                LuaValue::UserData(ud) => {
                    let r = ud.borrow::<RangeHandle>()?.clone();
                    out.extend(r.values(lua)?);
                }
                LuaValue::Nil => {}
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "range helpers take numbers or ranges, not a {}",
                        other.type_name()
                    )));
                }
            }
        }
        Ok(out)
    }
    let t = lua.create_table()?;
    t.set(
        "sum",
        lua.create_function(|lua, args: MultiValue| to_lua(lua, &sum_values(&gather(lua, args)?)))?,
    )?;
    t.set(
        "min",
        lua.create_function(|lua, args: MultiValue| {
            Ok(gather(lua, args)?
                .iter()
                .filter_map(Value::as_f64)
                .reduce(f64::min))
        })?,
    )?;
    t.set(
        "max",
        lua.create_function(|lua, args: MultiValue| {
            Ok(gather(lua, args)?
                .iter()
                .filter_map(Value::as_f64)
                .reduce(f64::max))
        })?,
    )?;
    Ok(t)
}
