// SPDX-License-Identifier: MIT OR Apache-2.0

//! Conversions between cell values and Lua values.

use decel_model::{CellError, ErrorKind, Value};
use mlua::{Lua, Value as LuaValue};

pub fn to_lua(lua: &Lua, v: &Value) -> mlua::Result<LuaValue> {
    Ok(match v {
        Value::Nil | Value::Error(_) => LuaValue::Nil,
        Value::Bool(b) => LuaValue::Boolean(*b),
        Value::Int(i) => LuaValue::Integer(*i),
        Value::Num(n) => LuaValue::Number(*n),
        Value::Str(s) => LuaValue::String(lua.create_string(s.as_bytes())?),
    })
}

/// Converts a hook's return value. Tables, functions and userdata are not
/// cell values (DESIGN §3.2).
pub fn from_lua(v: &LuaValue, member: &str) -> Result<Value, CellError> {
    Ok(match v {
        LuaValue::Nil => Value::Nil,
        LuaValue::Boolean(b) => Value::Bool(*b),
        LuaValue::Integer(i) => Value::Int(*i),
        LuaValue::Number(n) => Value::Num(*n),
        LuaValue::String(s) => Value::str(s.to_string_lossy().as_str()),
        other => {
            return Err(CellError {
                kind: ErrorKind::Type,
                message: format!(
                    "{member} must return a number, string or boolean, not a {}",
                    other.type_name()
                )
                .into(),
                member: Some(member.into()),
                line: None,
            });
        }
    })
}
