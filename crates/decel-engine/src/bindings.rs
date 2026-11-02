// SPDX-License-Identifier: MIT OR Apache-2.0

//! Lua bindings for the `fmt`, `parse` and `color` modules (DESIGN §4.5).

use mlua::{Lua, Table, Value as LuaValue};

use crate::stdlib::{color, fmt, parse};

/// Numbers accepted by formatters: Lua numbers, or nil (formats as "").
fn num(v: &LuaValue) -> Option<f64> {
    match v {
        LuaValue::Integer(i) => Some(*i as f64),
        LuaValue::Number(n) => Some(*n),
        _ => None,
    }
}

/// Decimal places (or significant figures) from Lua, clamped to what the
/// stdlib supports.
fn dp(v: Option<i64>, default: u32) -> u32 {
    v.map_or(default, |d| d.clamp(0, i64::from(fmt::MAX_DP)) as u32)
}

/// Formatters pass non-numbers through `tostring`, so `fmt.fixed(nil)` is
/// `"nil"` rather than an error that breaks a display hook.
fn passthrough(v: &LuaValue) -> mlua::Result<String> {
    v.to_string()
}

pub fn fmt_module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "fixed",
        lua.create_function(|_, (v, d): (LuaValue, Option<i64>)| match num(&v) {
            Some(n) => Ok(fmt::fixed(n, dp(d, 2))),
            None => passthrough(&v),
        })?,
    )?;
    t.set(
        "thousands",
        lua.create_function(|_, (v, d): (LuaValue, Option<i64>)| match num(&v) {
            Some(n) => Ok(fmt::thousands(n, dp(d, 0))),
            None => passthrough(&v),
        })?,
    )?;
    t.set(
        "percent",
        lua.create_function(|_, (v, d): (LuaValue, Option<i64>)| match num(&v) {
            Some(n) => Ok(fmt::percent(n, dp(d, 0))),
            None => passthrough(&v),
        })?,
    )?;
    t.set(
        "sci",
        lua.create_function(|_, (v, s): (LuaValue, Option<i64>)| match num(&v) {
            Some(n) => Ok(fmt::sci(n, dp(s, 3).max(1))),
            None => passthrough(&v),
        })?,
    )?;
    t.set(
        "si",
        lua.create_function(
            |_, (v, unit, s): (LuaValue, Option<String>, Option<i64>)| match num(&v) {
                Some(n) => Ok(fmt::si(n, unit.as_deref().unwrap_or(""), dp(s, 3).max(1))),
                None => passthrough(&v),
            },
        )?,
    )?;
    t.set(
        "currency",
        lua.create_function(|_, (v, sym, d): (LuaValue, Option<String>, Option<i64>)| {
            match num(&v) {
                Some(n) => Ok(fmt::currency(n, sym.as_deref().unwrap_or("$"), dp(d, 2))),
                None => passthrough(&v),
            }
        })?,
    )?;
    t.set(
        "dms",
        lua.create_function(
            |_, (v, kind, d): (LuaValue, Option<String>, Option<i64>)| match num(&v) {
                Some(n) => {
                    let kind = match kind.as_deref() {
                        Some("lat") => fmt::DmsKind::Lat,
                        Some("lon") => fmt::DmsKind::Lon,
                        None => fmt::DmsKind::Signed,
                        Some(other) => {
                            return Err(mlua::Error::runtime(format!(
                                "fmt.dms: kind must be \"lat\", \"lon\" or nil, not {other:?}"
                            )));
                        }
                    };
                    Ok(fmt::dms(n, kind, dp(d, 0)))
                }
                None => passthrough(&v),
            },
        )?,
    )?;
    t.set(
        "date",
        lua.create_function(|_, (v, p): (LuaValue, Option<String>)| match num(&v) {
            Some(n) => Ok(fmt::date(n, p.as_deref().unwrap_or("%Y-%m-%d"))),
            None => passthrough(&v),
        })?,
    )?;
    t.set(
        "duration",
        lua.create_function(|_, v: LuaValue| match num(&v) {
            Some(n) => Ok(fmt::duration(n)),
            None => passthrough(&v),
        })?,
    )?;
    Ok(t)
}

pub fn parse_module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "number",
        lua.create_function(|_, s: Option<String>| Ok(s.as_deref().and_then(parse::number)))?,
    )?;
    t.set(
        "dms",
        lua.create_function(|_, s: Option<String>| Ok(s.as_deref().and_then(parse::dms)))?,
    )?;
    t.set(
        "date",
        lua.create_function(|_, (s, p): (Option<String>, Option<String>)| {
            Ok(s.as_deref()
                .and_then(|s| parse::date(s, p.as_deref().unwrap_or("%Y-%m-%d"))))
        })?,
    )?;
    t.set(
        "currency",
        lua.create_function(|_, s: Option<String>| Ok(s.as_deref().and_then(parse::currency)))?,
    )?;
    Ok(t)
}

pub fn color_module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(
        "rgb",
        lua.create_function(|_, (r, g, b): (f64, f64, f64)| Ok(color::rgb(r, g, b)))?,
    )?;
    t.set(
        "hex",
        lua.create_function(|_, s: String| Ok(color::hex(&s)))?,
    )?;
    t.set(
        "lerp",
        lua.create_function(|_, (a, b, x): (String, String, f64)| Ok(color::lerp(&a, &b, x)))?,
    )?;
    t.set(
        "scale",
        lua.create_function(|_, (v, lo, hi, a, b): (f64, f64, f64, String, String)| {
            Ok(color::scale(v, lo, hi, &a, &b))
        })?,
    )?;
    Ok(t)
}
