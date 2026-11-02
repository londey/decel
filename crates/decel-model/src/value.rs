// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::MetaHash;

/// The category of a cell error (DESIGN §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorKind {
    Lua,
    Cycle,
    Timeout,
    Memory,
    Ref,
    Validation,
    Type,
}

impl ErrorKind {
    /// The short code shown in a cell, e.g. `#CYCLE`.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::Lua => "#LUA",
            ErrorKind::Cycle => "#CYCLE",
            ErrorKind::Timeout => "#TIMEOUT",
            ErrorKind::Memory => "#MEMORY",
            ErrorKind::Ref => "#REF",
            ErrorKind::Validation => "#INVALID",
            ErrorKind::Type => "#TYPE",
        }
    }
}

/// An error value held by a cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellError {
    pub kind: ErrorKind,
    pub message: Arc<str>,
    /// The metatable member that raised the error, if known.
    pub member: Option<Arc<str>>,
    /// 1-based line within that member's body, if known.
    pub line: Option<u32>,
}

impl CellError {
    pub fn new(kind: ErrorKind, message: impl Into<Arc<str>>) -> Self {
        Self {
            kind,
            message: message.into(),
            member: None,
            line: None,
        }
    }
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.code(), self.message)?;
        if let Some(member) = &self.member {
            write!(f, " (in {member}")?;
            if let Some(line) = self.line {
                write!(f, ", line {line}")?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }
}

/// A cell value (DESIGN §3.2).
///
/// Raw values (what a cell stores) are only ever `Bool`, `Int`, `Num` or
/// `Str`. `Nil` and `Error` appear only as computed (final) values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(Arc<str>),
    Error(CellError),
}

impl PartialEq for Value {
    /// Numbers compare by bit pattern so that `NaN == NaN`; this is identity,
    /// not arithmetic equality.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a.to_bits() == b.to_bits(),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Error(a), Value::Error(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

impl Value {
    pub fn str(s: impl Into<Arc<str>>) -> Self {
        Value::Str(s.into())
    }

    /// Whether this value may be stored as a cell's raw value.
    pub fn is_storable(&self) -> bool {
        matches!(
            self,
            Value::Bool(_) | Value::Int(_) | Value::Num(_) | Value::Str(_)
        )
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// Text that re-parses to the same value with the default parser.
    ///
    /// Floats use Rust's shortest round-trip formatting and always carry a
    /// `.` or exponent, so `3.0` stays a float. Strings that would parse as
    /// something else get a leading `'`.
    pub fn edit_text(&self) -> String {
        match self {
            Value::Nil => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Num(n) => format!("{n:?}"),
            Value::Str(s) => {
                if crate::parse_default(s).as_ref() != Some(self) {
                    format!("'{s}")
                } else {
                    s.to_string()
                }
            }
            Value::Error(e) => e.kind.code().to_string(),
        }
    }

    /// Default display text (no metatable `display` hook).
    pub fn display_text(&self) -> String {
        match self {
            Value::Nil => String::new(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Num(n) => format_number(*n),
            Value::Str(s) => s.to_string(),
            Value::Error(e) => e.kind.code().to_string(),
        }
    }
}

/// Formats a float for display with up to 10 significant digits, switching to
/// scientific notation for very large or very small magnitudes.
pub(crate) fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "∞" } else { "-∞" }.into();
    }
    if n == 0.0 {
        return "0".into();
    }
    let mag = n.abs();
    if !(1e-6..1e15).contains(&mag) {
        let s = format!("{n:.6e}");
        // Trim mantissa zeros: 1.500000e3 -> 1.5e3
        if let Some((mant, exp)) = s.split_once('e') {
            let mant = mant.trim_end_matches('0').trim_end_matches('.');
            return format!("{mant}e{exp}");
        }
        return s;
    }
    let digits = 10_i32;
    let int_digits = mag.log10().floor() as i32 + 1;
    let dp = (digits - int_digits).clamp(0, 15) as usize;
    let s = format!("{n:.dp$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// A cell: an optional raw value plus an optional metatable reference.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub raw: Option<Value>,
    pub meta: Option<MetaHash>,
}

impl Cell {
    pub const EMPTY: Cell = Cell {
        raw: None,
        meta: None,
    };

    pub fn value(v: Value) -> Self {
        Self {
            raw: Some(v),
            meta: None,
        }
    }

    pub fn with_meta(mut self, meta: MetaHash) -> Self {
        self.meta = Some(meta);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_none() && self.meta.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_equals_itself() {
        assert_eq!(Value::Num(f64::NAN), Value::Num(f64::NAN));
        assert_ne!(Value::Num(0.0), Value::Num(-0.0));
    }

    #[test]
    fn edit_text_round_trips() {
        for v in [
            Value::Int(-42),
            Value::Num(3.0),
            Value::Num(0.1),
            Value::Num(1e-7),
            Value::Num(-2.5e300),
            Value::Bool(true),
            Value::str("hello"),
            Value::str("42"),
            Value::str("true"),
            Value::str("'quoted"),
        ] {
            let text = v.edit_text();
            assert_eq!(crate::parse_default(&text), Some(v.clone()), "{text}");
        }
    }

    #[test]
    fn number_display() {
        assert_eq!(format_number(1.5), "1.5");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1234567.891), "1234567.891");
        assert_eq!(format_number(1e20), "1e20");
        assert_eq!(format_number(1.5e-9), "1.5e-9");
        assert_eq!(format_number(-0.000123), "-0.000123");
        assert_eq!(format_number(2.0 / 3.0), "0.6666666667");
    }
}
