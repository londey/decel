// SPDX-License-Identifier: MIT OR Apache-2.0

//! Metatable compilation (DESIGN §4.2, §5.1).
//!
//! Each member is compiled as its own chunk named after the member, so
//! syntax and runtime errors report that member and a line within its body.
//! Hooks are compiled as `return function(<signature>) <body>\nend` with the
//! body starting on line 1. The resulting members go into a table that is
//! then frozen: hooks receive the read-only proxy as `self`.

use std::collections::HashMap;
use std::sync::Arc;

use decel_model::{CellError, ErrorKind, MemberKind, MetaSource};
use mlua::{Function, RegistryKey, Table, Value as LuaValue};

use crate::core::MetaInfo;
use crate::display::CellStyle;
use crate::sandbox::{Fault, Sandbox, classify, freeze};

pub struct Compiled {
    /// The frozen metatable, passed to hooks as `self`.
    pub proxy: RegistryKey,
    /// Compiled hook functions, or the error that prevented compiling them.
    pub hooks: HashMap<String, Result<RegistryKey, CellError>>,
    /// Errors per member, in source order (empty if everything compiled).
    pub errors: Vec<(String, CellError)>,
    pub info: MetaInfo,
    pub style: CellStyle,
}

fn fault_error(f: Fault, member: &str) -> CellError {
    match f {
        Fault::Error(e) => e,
        Fault::NeedDep(_) => CellError {
            kind: ErrorKind::Lua,
            message: "fields and helpers cannot read cells".into(),
            member: Some(member.into()),
            line: None,
        },
    }
}

/// Clamps an error line to the member's body (a missing `end` is reported on
/// the line after the body).
fn clamp_line(mut e: CellError, body_lines: u32) -> CellError {
    if let Some(l) = e.line {
        e.line = Some(l.clamp(1, body_lines.max(1)));
    }
    e
}

pub fn compile(sb: &Sandbox, src: &MetaSource) -> mlua::Result<Compiled> {
    let lua = sb.lua();
    let table = lua.create_table()?;
    let mut hooks = HashMap::new();
    let mut errors = Vec::new();
    for m in &src.members {
        let body_lines = m.source.lines().count().max(1) as u32;
        let chunk = match (m.kind, m.signature()) {
            (MemberKind::Hook, Some(params)) => {
                format!("return function({params}) {}\nend", m.source)
            }
            _ => format!("return {}", m.source.trim_end()),
        };
        let compiled = sb
            .load(&m.name, &chunk)
            .map_err(|e| fault_error(classify(&e, &m.name), &m.name))
            .and_then(|f| {
                let (r, _) = sb.run(|_| f.call::<LuaValue>(()));
                r.map_err(|e| fault_error(classify(&e, &m.name), &m.name))
            })
            .map_err(|e| clamp_line(e, body_lines));
        match compiled {
            Ok(value) => {
                if m.kind == MemberKind::Hook {
                    match &value {
                        LuaValue::Function(f) => {
                            hooks.insert(m.name.clone(), Ok(lua.create_registry_value(f.clone())?));
                        }
                        _ => {
                            let e = CellError {
                                kind: ErrorKind::Lua,
                                message: "hook did not compile to a function".into(),
                                member: Some(m.name.clone().into()),
                                line: None,
                            };
                            hooks.insert(m.name.clone(), Err(e.clone()));
                            errors.push((m.name.clone(), e));
                        }
                    }
                }
                table.raw_set(m.name.as_str(), value)?;
            }
            Err(e) => {
                if m.kind == MemberKind::Hook {
                    hooks.insert(m.name.clone(), Err(e.clone()));
                }
                errors.push((m.name.clone(), e));
            }
        }
    }
    let label = |name: &str| -> Option<Arc<str>> {
        match table.raw_get::<LuaValue>(name) {
            Ok(LuaValue::String(s)) => Some(s.to_string_lossy().as_str().into()),
            _ => None,
        }
    };
    let info = MetaInfo {
        kind: label("kind"),
        unit: label("unit"),
        has_value: hooks.contains_key("value"),
    };
    let style = match table.raw_get::<LuaValue>("style") {
        Ok(LuaValue::Table(t)) => CellStyle::from_table(&t),
        _ => CellStyle::default(),
    };
    let proxy = freeze(lua, table)?;
    Ok(Compiled {
        proxy: lua.create_registry_value(proxy)?,
        hooks,
        errors,
        info,
        style,
    })
}

/// Fetches a compiled hook function and the proxy table.
pub fn hook(
    sb: &Sandbox,
    c: &Compiled,
    name: &str,
) -> Option<Result<(Function, Table), CellError>> {
    let key = match c.hooks.get(name)? {
        Ok(k) => k,
        Err(e) => return Some(Err(e.clone())),
    };
    let lua = sb.lua();
    let pair = lua
        .registry_value::<Function>(key)
        .and_then(|f| Ok((f, lua.registry_value::<Table>(&c.proxy)?)));
    Some(pair.map_err(|e| CellError::new(ErrorKind::Lua, e.to_string())))
}
