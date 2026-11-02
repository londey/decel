// SPDX-License-Identifier: MIT OR Apache-2.0

//! Encoding of values, cells and change records to and from SQLite.

use decel_model::{
    Axis, Cell, CellAddr, Change, ChartId, ChartRecord, MetaHash, SheetId, SheetRecord, StructOp,
    Value,
};
use rusqlite::types::{Value as SqlValue, ValueRef};

use crate::StoreError;

const VT_BOOL: i64 = 1;
const VT_INT: i64 = 2;
const VT_NUM: i64 = 3;
const VT_STR: i64 = 4;

/// `(vtype, value)` for a raw value. NaN is stored as text because SQLite
/// turns NaN reals into NULL.
pub fn encode_raw(raw: &Option<Value>) -> Result<(Option<i64>, SqlValue), StoreError> {
    Ok(match raw {
        None => (None, SqlValue::Null),
        Some(Value::Bool(b)) => (Some(VT_BOOL), SqlValue::Integer(i64::from(*b))),
        Some(Value::Int(i)) => (Some(VT_INT), SqlValue::Integer(*i)),
        Some(Value::Num(n)) if n.is_nan() => (Some(VT_NUM), SqlValue::Text("NaN".into())),
        Some(Value::Num(n)) => (Some(VT_NUM), SqlValue::Real(*n)),
        Some(Value::Str(s)) => (Some(VT_STR), SqlValue::Text(s.to_string())),
        Some(Value::Nil | Value::Error(_)) => return Err(StoreError::InvalidRaw),
    })
}

pub fn decode_raw(vtype: Option<i64>, value: ValueRef<'_>) -> Result<Option<Value>, StoreError> {
    let Some(vtype) = vtype else {
        return Ok(None);
    };
    let v = match (vtype, value) {
        (VT_BOOL, ValueRef::Integer(i)) => Value::Bool(i != 0),
        (VT_INT, ValueRef::Integer(i)) => Value::Int(i),
        (VT_NUM, ValueRef::Real(f)) => Value::Num(f),
        (VT_NUM, ValueRef::Integer(i)) => Value::Num(i as f64),
        (VT_NUM, ValueRef::Text(t)) => {
            let s = std::str::from_utf8(t).map_err(|_| StoreError::Corrupt("bad number text"))?;
            Value::Num(
                s.parse()
                    .map_err(|_| StoreError::Corrupt("bad number text"))?,
            )
        }
        (VT_STR, ValueRef::Text(t)) => {
            Value::str(std::str::from_utf8(t).map_err(|_| StoreError::Corrupt("bad utf-8 string"))?)
        }
        _ => return Err(StoreError::Corrupt("value does not match vtype")),
    };
    Ok(Some(v))
}

pub fn encode_meta(meta: &Option<MetaHash>) -> Option<Vec<u8>> {
    meta.map(|h| h.0.to_vec())
}

pub fn decode_meta(blob: Option<Vec<u8>>) -> Result<Option<MetaHash>, StoreError> {
    blob.map(|b| MetaHash::from_slice(&b).ok_or(StoreError::Corrupt("bad meta hash")))
        .transpose()
}

pub fn decode_cell(
    vtype: Option<i64>,
    value: ValueRef<'_>,
    meta: Option<Vec<u8>>,
) -> Result<Cell, StoreError> {
    Ok(Cell {
        raw: decode_raw(vtype, value)?,
        meta: decode_meta(meta)?,
    })
}

/// An `other_changes` record: entity key plus JSON before/after.
pub struct OtherRecord {
    pub entity: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

fn json<T: serde::Serialize>(v: &Option<T>) -> Result<Option<String>, StoreError> {
    v.as_ref()
        .map(|v| serde_json::to_string(v).map_err(|_| StoreError::Corrupt("json encode")))
        .transpose()
}

fn unjson<T: serde::de::DeserializeOwned>(s: &Option<String>) -> Result<Option<T>, StoreError> {
    s.as_deref()
        .map(|s| serde_json::from_str(s).map_err(|_| StoreError::Corrupt("json decode")))
        .transpose()
}

/// Encodes a non-cell change. Panics are impossible; cell changes return `None`.
pub fn encode_other(change: &Change) -> Result<Option<OtherRecord>, StoreError> {
    Ok(Some(match change {
        Change::Cell { .. } => return Ok(None),
        Change::Sheet { id, before, after } => OtherRecord {
            entity: format!("sheet:{id}"),
            before: json(before)?,
            after: json(after)?,
        },
        Change::Layout {
            sheet,
            axis,
            idx,
            before,
            after,
        } => OtherRecord {
            entity: format!("layout:{sheet}:{}:{idx}", axis.to_int()),
            before: json(before)?,
            after: json(after)?,
        },
        Change::Chart { id, before, after } => OtherRecord {
            entity: format!("chart:{}", id.0),
            before: json(before)?,
            after: json(after)?,
        },
        Change::Structure(op) => OtherRecord {
            entity: "structure".into(),
            before: None,
            after: json(&Some(*op))?,
        },
    }))
}

pub fn decode_other(rec: &OtherRecord) -> Result<Change, StoreError> {
    let bad = || StoreError::Corrupt("bad entity key");
    let mut parts = rec.entity.split(':');
    let kind = parts.next().ok_or_else(bad)?;
    let mut num =
        || -> Result<u32, StoreError> { parts.next().and_then(|p| p.parse().ok()).ok_or_else(bad) };
    Ok(match kind {
        "sheet" => Change::Sheet {
            id: SheetId(num()?),
            before: unjson::<SheetRecord>(&rec.before)?,
            after: unjson::<SheetRecord>(&rec.after)?,
        },
        "layout" => {
            let sheet = SheetId(num()?);
            let axis = Axis::from_int(i64::from(num()?)).ok_or_else(bad)?;
            let idx = num()?;
            Change::Layout {
                sheet,
                axis,
                idx,
                before: unjson::<f64>(&rec.before)?,
                after: unjson::<f64>(&rec.after)?,
            }
        }
        "chart" => Change::Chart {
            id: ChartId(num()?),
            before: unjson::<ChartRecord>(&rec.before)?,
            after: unjson::<ChartRecord>(&rec.after)?,
        },
        "structure" => Change::Structure(unjson::<StructOp>(&rec.after)?.ok_or_else(bad)?),
        _ => return Err(bad()),
    })
}

pub fn addr(sheet: i64, row: i64, col: i64) -> Result<CellAddr, StoreError> {
    let c = |v: i64| u32::try_from(v).map_err(|_| StoreError::Corrupt("coordinate out of range"));
    Ok(CellAddr::new(SheetId(c(sheet)?), c(row)?, c(col)?))
}
