// SPDX-License-Identifier: MIT OR Apache-2.0

//! Evaluation state shared with Lua callbacks.
//!
//! The engine owns a [`Core`] and lends it to the Lua VM (as app data) for
//! the duration of each hook call, so handle methods can read cells and
//! record dependencies without unsafe code.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use decel_model::{Cell, CellAddr, MetaHash, Value, Workbook};

use crate::deps::Deps;

/// Static facts about a compiled metatable that callbacks need.
#[derive(Debug, Clone, Default)]
pub struct MetaInfo {
    pub kind: Option<Arc<str>>,
    pub unit: Option<Arc<str>>,
    /// What the code side shows: `kind`, or the first member's name.
    pub label: Option<Arc<str>>,
    pub has_value: bool,
}

/// The cell currently being evaluated.
#[derive(Debug, Default)]
pub struct Frame {
    pub addr: Option<CellAddr>,
    /// Whether reads are recorded as dependencies.
    pub record: bool,
    pub deps: Deps,
    /// Overrides the evaluating cell's metatable info (previews).
    pub override_info: Option<MetaInfo>,
}

#[derive(Default)]
pub struct Core {
    pub wb: Workbook,
    pub info: HashMap<MetaHash, MetaInfo>,
    pub values: HashMap<CellAddr, Value>,
    pub dirty: BTreeSet<CellAddr>,
    pub frame: Frame,
}

/// Result of reading a cell's final value from a callback.
pub enum Read {
    Ready(Value),
    /// The cell is computed and not yet evaluated.
    NeedDep(CellAddr),
}

impl Core {
    pub fn cell(&self, a: CellAddr) -> Option<&Cell> {
        self.wb.cell(a)
    }

    pub fn meta_info(&self, a: CellAddr) -> Option<&MetaInfo> {
        if self.frame.addr == Some(a)
            && let Some(info) = &self.frame.override_info
        {
            return Some(info);
        }
        let hash = self.cell(a)?.meta?;
        self.info.get(&hash)
    }

    pub fn is_computed(&self, a: CellAddr) -> bool {
        self.cell(a)
            .and_then(|c| c.meta)
            .and_then(|h| self.info.get(&h))
            .is_some_and(|i| i.has_value)
    }

    pub fn needs_eval(&self, a: CellAddr) -> bool {
        self.is_computed(a) && (self.dirty.contains(&a) || !self.values.contains_key(&a))
    }

    /// The final value of `a`: the computed value, or the raw value.
    pub fn read(&self, a: CellAddr) -> Read {
        if self.is_computed(a) {
            if self.needs_eval(a) {
                return Read::NeedDep(a);
            }
            return Read::Ready(self.values.get(&a).cloned().unwrap_or(Value::Nil));
        }
        Read::Ready(
            self.cell(a)
                .and_then(|c| c.raw.clone())
                .unwrap_or(Value::Nil),
        )
    }

    pub fn record_cell(&mut self, a: CellAddr) {
        if self.frame.record && self.frame.addr != Some(a) {
            self.frame.deps.cell(a);
        }
    }
}
