// SPDX-License-Identifier: MIT OR Apache-2.0

//! The headless engine: compiles metatables, tracks dependencies, recalculates
//! and runs the value/display/edit pipelines (DESIGN §3.3, §6).
//!
//! Evaluation never recurses through Lua: when a hook reads a dependency that
//! is not yet evaluated, the hook is interrupted, the dependency is evaluated
//! from an explicit work stack, and the hook is retried. That keeps long
//! dependency chains (a 500-row running total) off the C stack and makes
//! cycle detection a stack lookup.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use decel_model::{
    Cell, CellAddr, CellError, CellStorage, Change, ErrorKind, MetaHash, MetaSource, Value,
    Workbook, parse_default,
};
use mlua::{MultiValue, Value as LuaValue};

use crate::api::{Ctx, range_module};
use crate::bindings::{color_module, fmt_module, parse_module};
use crate::compile::{Compiled, compile, hook};
use crate::convert::{from_lua, to_lua};
use crate::core::{Core, Frame, MetaInfo};
use crate::deps::{DepGraph, Deps};
use crate::display::{Align, CellStyle, DisplayCell};
use crate::sandbox::{Fault, Limits, Sandbox, classify};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("lua: {0}")]
    Lua(#[from] mlua::Error),
}

/// Why an edit was refused (parse or validation failure).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct EditRejected {
    pub message: String,
}

/// Status of one member in a preview or compile report.
#[derive(Debug, Clone, PartialEq)]
pub enum MemberStatus {
    Ok,
    /// Failed to compile.
    Syntax(CellError),
    /// Compiled, but raised an error when run against the previewed cell.
    Runtime(CellError),
}

/// The result of running an uncommitted metatable against a cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub members: Vec<(String, MemberStatus)>,
    pub value: Value,
    pub display: DisplayCell,
    pub log: Vec<String>,
}

/// Arguments for a hook call after `self, c`.
enum Args<'a> {
    None,
    Value(&'a Value),
    Text(&'a str),
}

/// Outcome of evaluating one cell's `value` hook.
enum Step {
    Done,
    Need(CellAddr),
}

/// Maximum dependency retries for a single non-value hook call.
const MAX_RETRIES: usize = 100_000;

pub struct Engine {
    sb: Sandbox,
    core: Core,
    compiled: HashMap<MetaHash, Compiled>,
    graph: DepGraph,
    display_graph: DepGraph,
    /// Deps recorded by an interrupted evaluation, kept in case it turns out
    /// to be part of a cycle.
    partial: HashMap<CellAddr, Deps>,
    warnings: HashMap<CellAddr, Arc<str>>,
    logs: HashMap<CellAddr, Vec<String>>,
    /// Cells whose display may have changed since the last `take_changed`.
    changed: HashSet<CellAddr>,
}

impl Engine {
    /// Builds an engine over a workbook and marks every computed cell dirty.
    /// Call [`Engine::recalc`] to evaluate.
    pub fn new(wb: Workbook, limits: Limits) -> Result<Self, EngineError> {
        let sb = Sandbox::new(limits)?;
        sb.add_module("fmt", fmt_module(sb.lua())?)?;
        sb.add_module("parse", parse_module(sb.lua())?)?;
        sb.add_module("color", color_module(sb.lua())?)?;
        sb.add_module("range", range_module(sb.lua())?)?;
        let mut engine = Self {
            sb,
            core: Core::default(),
            compiled: HashMap::new(),
            graph: DepGraph::default(),
            display_graph: DepGraph::default(),
            partial: HashMap::new(),
            warnings: HashMap::new(),
            logs: HashMap::new(),
            changed: HashSet::new(),
        };
        let metas: Vec<Arc<MetaSource>> = wb.metas.values().cloned().collect();
        engine.core.wb = wb;
        for m in metas {
            engine.ensure_compiled(&m)?;
        }
        engine.mark_all_dirty();
        Ok(engine)
    }

    pub fn workbook(&self) -> &Workbook {
        &self.core.wb
    }

    /// Updates a workbook setting in the engine's copy of the workbook.
    pub fn set_setting(&mut self, key: &str, value: &str) {
        self.core
            .wb
            .settings
            .insert(key.to_string(), value.to_string());
    }

    pub fn limits(&self) -> Limits {
        self.sb.limits()
    }

    /// Compiles a metatable (once per hash) and registers its source.
    pub fn add_meta(&mut self, src: &MetaSource) -> Result<MetaHash, EngineError> {
        let hash = self.ensure_compiled(src)?;
        self.core.wb.add_meta(src.clone());
        Ok(hash)
    }

    fn ensure_compiled(&mut self, src: &MetaSource) -> Result<MetaHash, EngineError> {
        let hash = src.hash();
        if !self.compiled.contains_key(&hash) {
            let c = compile(&self.sb, src)?;
            self.core.info.insert(hash, c.info.clone());
            self.compiled.insert(hash, c);
        }
        Ok(hash)
    }

    /// Per-member compile errors for a metatable.
    pub fn compile_errors(&self, hash: &MetaHash) -> Vec<(String, CellError)> {
        self.compiled
            .get(hash)
            .map(|c| c.errors.clone())
            .unwrap_or_default()
    }

    pub fn meta_info(&self, hash: &MetaHash) -> Option<&MetaInfo> {
        self.core.info.get(hash)
    }

    // -----------------------------------------------------------------------
    // Applying changes

    /// Applies committed changes (from the store) and marks what they affect
    /// dirty. Metatables referenced by the changes must already be known
    /// (see [`Engine::add_meta`]).
    pub fn apply(&mut self, changes: &[Change]) {
        let mut seeds: Vec<CellAddr> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        let mut rebuild = false;
        for change in changes {
            match change {
                Change::Cell {
                    addr,
                    before,
                    after,
                } => {
                    self.core.wb.apply_change(change);
                    if before != after {
                        seeds.push(*addr);
                        self.reset_cell(*addr);
                    }
                }
                Change::Sheet { before, after, .. } => {
                    if let Some(b) = before {
                        names.push(b.name.clone());
                    }
                    if let Some(a) = after {
                        names.push(a.name.clone());
                    }
                    self.core.wb.apply_change(change);
                }
                Change::Structure(_) => {
                    self.core.wb.apply_change(change);
                    rebuild = true;
                }
                other => self.core.wb.apply_change(other),
            }
        }
        if rebuild {
            self.mark_all_dirty();
            return;
        }
        // Cells that looked a sheet up by name are themselves affected.
        let mut by_name = Vec::new();
        for n in names {
            self.graph.name_dependents(&n, &mut by_name);
            self.display_graph.name_dependents(&n, &mut by_name);
        }
        for a in by_name {
            if self.core.is_computed(a) {
                self.core.dirty.insert(a);
            }
            seeds.push(a);
        }
        self.mark_dirty(seeds);
    }

    /// Forgets cached results for a cell whose raw value or meta changed.
    fn reset_cell(&mut self, a: CellAddr) {
        self.core.values.remove(&a);
        self.warnings.remove(&a);
        self.graph.remove(a);
        self.display_graph.remove(a);
        self.logs.remove(&a);
        if self.core.is_computed(a) {
            self.core.dirty.insert(a);
        } else {
            self.core.dirty.remove(&a);
        }
        self.changed.insert(a);
    }

    /// Discards every computed result and marks all computed cells dirty.
    pub fn mark_all_dirty(&mut self) {
        self.graph.clear();
        self.display_graph.clear();
        self.core.values.clear();
        self.warnings.clear();
        self.partial.clear();
        self.core.dirty.clear();
        let mut all = Vec::new();
        for sheet in self.core.wb.sheets.values() {
            for ((r, c), _) in sheet.cells.iter() {
                all.push(CellAddr::new(sheet.id, r, c));
            }
        }
        for a in all {
            if self.core.is_computed(a) {
                self.core.dirty.insert(a);
            }
            self.changed.insert(a);
        }
    }

    /// Marks the transitive dependents of `seeds` dirty (and their displays
    /// changed).
    fn mark_dirty(&mut self, seeds: Vec<CellAddr>) {
        let mut queue = seeds;
        let mut seen: HashSet<CellAddr> = queue.iter().copied().collect();
        let mut next = Vec::new();
        while let Some(a) = queue.pop() {
            self.changed.insert(a);
            next.clear();
            self.graph.dependents(a, &mut next);
            for &d in &next {
                if self.core.is_computed(d) {
                    self.core.dirty.insert(d);
                }
                if seen.insert(d) {
                    queue.push(d);
                }
            }
            // Display hooks that read `a` re-render, but values don't change.
            next.clear();
            self.display_graph.dependents(a, &mut next);
            self.changed.extend(next.iter().copied());
        }
    }

    // -----------------------------------------------------------------------
    // Recalculation

    /// Cells still waiting to be evaluated.
    pub fn pending(&self) -> usize {
        self.core.dirty.len()
    }

    /// Evaluates dirty cells until none remain or `stop` returns true.
    /// Returns the number still pending.
    pub fn recalc(&mut self, mut stop: impl FnMut() -> bool) -> usize {
        while let Some(&next) = self.core.dirty.first() {
            if stop() {
                break;
            }
            self.ensure(next);
        }
        self.core.dirty.len()
    }

    /// Recalculates everything that is dirty.
    pub fn recalc_all(&mut self) {
        self.recalc(|| false);
    }

    /// Evaluates `start` and whatever it needs, using an explicit stack.
    fn ensure(&mut self, start: CellAddr) {
        let mut stack = vec![start];
        while let Some(&top) = stack.last() {
            if !self.core.needs_eval(top) {
                stack.pop();
                continue;
            }
            match self.eval_value(top) {
                Step::Done => {
                    stack.pop();
                }
                Step::Need(dep) => match stack.iter().position(|s| *s == dep) {
                    Some(p) => {
                        let members = stack.split_off(p);
                        self.mark_cycle(&members);
                    }
                    None => stack.push(dep),
                },
            }
        }
    }

    fn mark_cycle(&mut self, members: &[CellAddr]) {
        for &m in members {
            let deps = self.partial.remove(&m).unwrap_or_default();
            self.graph.set(m, deps);
            let e = CellError::new(
                ErrorKind::Cycle,
                format!("circular reference through {} cell(s)", members.len()),
            );
            self.store_value(m, Value::Error(e));
        }
    }

    fn store_value(&mut self, a: CellAddr, v: Value) {
        self.core.dirty.remove(&a);
        let old = self.core.values.insert(a, v.clone());
        if old.as_ref() != Some(&v) {
            self.changed.insert(a);
        }
    }

    /// Runs `value` (then `validate`) for one cell.
    fn eval_value(&mut self, a: CellAddr) -> Step {
        let Some(hash) = self.core.cell(a).and_then(|c| c.meta) else {
            self.core.dirty.remove(&a);
            return Step::Done;
        };
        let (result, deps, log) = self.call(a, hash, "value", Args::None, true, None);
        let value = match result {
            Some(Ok(mut values)) => {
                let v = values.pop_front().unwrap_or(LuaValue::Nil);
                from_lua(&v, "value").unwrap_or_else(Value::Error)
            }
            Some(Err(Fault::NeedDep(d))) => {
                self.partial.insert(a, deps);
                return Step::Need(d);
            }
            Some(Err(Fault::Error(e))) => Value::Error(e),
            None => Value::Nil,
        };
        let mut deps = deps;
        let mut warning = None;
        if !value.is_error() {
            let (res, vdeps, _) = self.call(a, hash, "validate", Args::Value(&value), true, None);
            match res {
                Some(Err(Fault::NeedDep(d))) => {
                    deps.cells.extend(vdeps.cells);
                    self.partial.insert(a, deps);
                    return Step::Need(d);
                }
                Some(r) => {
                    deps.cells.extend(vdeps.cells);
                    deps.ranges.extend(vdeps.ranges);
                    deps.names.extend(vdeps.names);
                    warning = validation_message(r).map(Arc::from);
                }
                None => {}
            }
        }
        self.partial.remove(&a);
        self.graph.set(a, deps);
        match warning {
            Some(w) => {
                self.warnings.insert(a, w);
            }
            None => {
                self.warnings.remove(&a);
            }
        }
        if !log.is_empty() {
            self.logs.insert(a, log);
        } else {
            self.logs.remove(&a);
        }
        self.store_value(a, value);
        Step::Done
    }

    /// Calls a hook. Returns `None` if the metatable lacks it.
    #[allow(clippy::type_complexity)]
    fn call(
        &mut self,
        a: CellAddr,
        hash: MetaHash,
        name: &str,
        args: Args<'_>,
        record: bool,
        override_info: Option<MetaInfo>,
    ) -> (Option<Result<MultiValue, Fault>>, Deps, Vec<String>) {
        let Some(compiled) = self.compiled.get(&hash) else {
            return (None, Deps::default(), Vec::new());
        };
        let (f, proxy) = match hook(&self.sb, compiled, name) {
            None => return (None, Deps::default(), Vec::new()),
            Some(Err(e)) => return (Some(Err(Fault::Error(e))), Deps::default(), Vec::new()),
            Some(Ok(pair)) => pair,
        };
        self.core.frame = Frame {
            addr: Some(a),
            record,
            deps: Deps::default(),
            override_info,
        };
        let lua = self.sb.lua();
        lua.set_app_data(std::mem::take(&mut self.core));
        let (result, log) = self.sb.run(|lua| {
            let c = Ctx(a);
            match args {
                Args::None => f.call::<MultiValue>((proxy, c)),
                Args::Value(v) => f.call::<MultiValue>((proxy, c, to_lua(lua, v)?)),
                Args::Text(t) => f.call::<MultiValue>((proxy, c, t)),
            }
        });
        self.core = lua.remove_app_data::<Core>().unwrap_or_default();
        let deps = std::mem::take(&mut self.core.frame.deps);
        self.core.frame = Frame::default();
        (Some(result.map_err(|e| classify(&e, name))), deps, log)
    }

    /// Calls a non-value hook, evaluating any dependencies it needs first.
    fn call_resolved(
        &mut self,
        a: CellAddr,
        hash: MetaHash,
        name: &str,
        args: Args<'_>,
        record: bool,
        override_info: Option<MetaInfo>,
    ) -> (Option<Result<MultiValue, CellError>>, Deps, Vec<String>) {
        for _ in 0..MAX_RETRIES {
            let args = match &args {
                Args::None => Args::None,
                Args::Value(v) => Args::Value(v),
                Args::Text(t) => Args::Text(t),
            };
            let (r, deps, log) = self.call(a, hash, name, args, record, override_info.clone());
            match r {
                Some(Err(Fault::NeedDep(d))) => self.ensure(d),
                Some(Err(Fault::Error(e))) => return (Some(Err(e)), deps, log),
                Some(Ok(v)) => return (Some(Ok(v)), deps, log),
                None => return (None, deps, log),
            }
        }
        let e = CellError::new(ErrorKind::Lua, "too many dependency retries");
        (Some(Err(e)), Deps::default(), Vec::new())
    }

    // -----------------------------------------------------------------------
    // Queries and pipelines

    /// The final value of a cell, evaluating it if needed.
    pub fn final_value(&mut self, a: CellAddr) -> Value {
        if self.core.needs_eval(a) {
            self.ensure(a);
        }
        match self.core.read(a) {
            crate::core::Read::Ready(v) => v,
            crate::core::Read::NeedDep(_) => Value::Nil,
        }
    }

    pub fn warning(&self, a: CellAddr) -> Option<Arc<str>> {
        self.warnings.get(&a).cloned()
    }

    /// `print` output from the cell's last `value` evaluation.
    pub fn log(&self, a: CellAddr) -> &[String] {
        self.logs.get(&a).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Takes the set of cells whose display may have changed.
    pub fn take_changed(&mut self) -> HashSet<CellAddr> {
        std::mem::take(&mut self.changed)
    }

    fn compiled_style(&self, hash: Option<MetaHash>) -> CellStyle {
        hash.and_then(|h| self.compiled.get(&h))
            .map(|c| c.style.clone())
            .unwrap_or_default()
    }

    /// Display for a cell (DESIGN §3.3, §4.4).
    pub fn display(&mut self, a: CellAddr) -> DisplayCell {
        let hash = self.core.cell(a).and_then(|c| c.meta);
        let v = self.final_value(a);
        let info = hash.and_then(|h| self.core.info.get(&h)).cloned();
        let mut out = self.render(a, hash, &v, info.as_ref(), None);
        out.warning = self.warning(a);
        out
    }

    /// Shared display rendering for stored cells and previews.
    fn render(
        &mut self,
        a: CellAddr,
        hash: Option<MetaHash>,
        v: &Value,
        info: Option<&MetaInfo>,
        override_info: Option<MetaInfo>,
    ) -> DisplayCell {
        let mut out = DisplayCell {
            hash,
            kind: info.and_then(|i| i.kind.clone()),
            unit: info.and_then(|i| i.unit.clone()),
            label: info.and_then(|i| i.label.clone()),
            ..DisplayCell::default()
        };
        let default_style = CellStyle {
            align: Some(if v.as_f64().is_some() {
                Align::Right
            } else {
                Align::Left
            }),
            ..CellStyle::default()
        };
        let static_style = self.compiled_style(hash);
        if let Value::Error(e) = v {
            out.text = e.kind.code().to_string();
            out.error = Some(e.clone());
            out.style = static_style.or(&default_style);
            return out;
        }
        let mut hook_style = CellStyle::default();
        out.text = v.display_text();
        // Display hooks never see nil: an empty cell stays empty.
        if let Some(h) = hash.filter(|_| !matches!(v, Value::Nil)) {
            let record = override_info.is_none();
            let (r, deps, _) =
                self.call_resolved(a, h, "display", Args::Value(v), record, override_info);
            if record {
                self.display_graph.set(a, deps);
            }
            match r {
                None => {}
                Some(Ok(mut vals)) => match vals.pop_front().unwrap_or(LuaValue::Nil) {
                    LuaValue::Table(t) => {
                        if let Ok(text) = t.get::<LuaValue>("text") {
                            out.text = text.to_string().unwrap_or_default();
                            if matches!(text, LuaValue::Nil) {
                                out.text = v.display_text();
                            }
                        }
                        hook_style = CellStyle::from_table(&t);
                    }
                    LuaValue::Nil => {}
                    other => out.text = other.to_string().unwrap_or_default(),
                },
                Some(Err(e)) => out.display_error = Some(e),
            }
        }
        out.style = hook_style.or(&static_style).or(&default_style);
        out
    }

    /// Hover text (DESIGN §8.5).
    pub fn tooltip(&mut self, a: CellAddr) -> Option<String> {
        let cell = self.core.cell(a).cloned()?;
        let v = self.final_value(a);
        let mut text = if let Value::Error(e) = &v {
            Some(e.to_string())
        } else {
            let hooked = cell
                .meta
                .filter(|_| !matches!(v, Value::Nil))
                .and_then(|h| {
                    let (r, _, _) =
                        self.call_resolved(a, h, "tooltip", Args::Value(&v), false, None);
                    match r? {
                        Ok(mut vals) => match vals.pop_front().unwrap_or(LuaValue::Nil) {
                            LuaValue::Nil => Some(None),
                            other => Some(other.to_string().ok()),
                        },
                        Err(e) => Some(Some(format!("tooltip failed: {e}"))),
                    }
                });
            match hooked {
                Some(t) => t,
                None => {
                    let unit = cell
                        .meta
                        .and_then(|h| self.core.info.get(&h))
                        .and_then(|i| i.unit.clone());
                    match (&v, unit) {
                        (Value::Nil, _) => None,
                        (v, Some(u)) => Some(format!("{} {u}", v.display_text())),
                        (v, None) => Some(v.display_text()),
                    }
                }
            }
        };
        if let Some(w) = self.warning(a) {
            text = Some(match text {
                Some(t) => format!("{t}\n⚠ {w}"),
                None => format!("⚠ {w}"),
            });
        }
        text
    }

    /// Text placed in the editor when editing starts.
    pub fn edit_text(&mut self, a: CellAddr) -> String {
        let Some(cell) = self.core.cell(a).cloned() else {
            return String::new();
        };
        let raw = cell.raw.clone().unwrap_or(Value::Nil);
        if let Some(h) = cell.meta.filter(|_| !matches!(raw, Value::Nil)) {
            let (r, _, _) = self.call_resolved(a, h, "edit", Args::Value(&raw), false, None);
            if let Some(Ok(mut vals)) = r
                && let Some(v) = vals.pop_front()
                && !matches!(v, LuaValue::Nil)
            {
                return v.to_string().unwrap_or_default();
            }
        }
        raw.edit_text()
    }

    /// Hint shown in the edit bar while editing.
    pub fn edit_hint(&mut self, a: CellAddr) -> Option<String> {
        let h = self.core.cell(a)?.meta?;
        let (r, _, _) = self.call_resolved(a, h, "edit_hint", Args::None, false, None);
        match r? {
            Ok(mut vals) => match vals.pop_front()? {
                LuaValue::Nil => None,
                v => v.to_string().ok(),
            },
            Err(_) => None,
        }
    }

    /// Parses and validates typed text for a cell (DESIGN §3.3). Returns the
    /// cell to commit; nothing is changed here.
    pub fn enter_text(&mut self, a: CellAddr, text: &str) -> Result<Cell, EditRejected> {
        let meta = self.core.cell(a).and_then(|c| c.meta);
        let raw = match meta {
            Some(h) => {
                let (r, _, _) = self.call_resolved(a, h, "parse", Args::Text(text), false, None);
                match r {
                    None => parse_default(text),
                    Some(Err(e)) => return Err(reject(format!("parse failed: {}", e.message))),
                    Some(Ok(mut vals)) => {
                        let first = vals.pop_front().unwrap_or(LuaValue::Nil);
                        let second = vals.pop_front().unwrap_or(LuaValue::Nil);
                        match (&first, second) {
                            (LuaValue::Nil, LuaValue::Nil) => None,
                            (LuaValue::Nil, msg) => {
                                return Err(reject(msg.to_string().unwrap_or_default()));
                            }
                            _ => {
                                let v = from_lua(&first, "parse")
                                    .map_err(|e| reject(e.message.to_string()))?;
                                Some(v)
                            }
                        }
                    }
                }
            }
            None => parse_default(text),
        };
        if let (Some(h), Some(v)) = (meta, &raw) {
            let (r, _, _) = self.call_resolved(a, h, "validate", Args::Value(v), false, None);
            if let Some(r) = r {
                let r = r.map_err(Fault::Error);
                if let Some(msg) = validation_message(r) {
                    return Err(reject(msg));
                }
            }
        }
        Ok(Cell { raw, meta })
    }

    /// Runs an uncommitted metatable against a cell without changing anything.
    pub fn preview(&mut self, a: CellAddr, src: &MetaSource) -> Result<Preview, EngineError> {
        let hash = self.ensure_compiled(src)?;
        let info = self.core.info.get(&hash).cloned().unwrap_or_default();
        let mut members: Vec<(String, MemberStatus)> = src
            .members
            .iter()
            .map(|m| (m.name.clone(), MemberStatus::Ok))
            .collect();
        if let Some(c) = self.compiled.get(&hash) {
            for (name, e) in &c.errors {
                if let Some(slot) = members.iter_mut().find(|(n, _)| n == name) {
                    slot.1 = MemberStatus::Syntax(e.clone());
                }
            }
        }
        let raw = self
            .core
            .cell(a)
            .and_then(|c| c.raw.clone())
            .unwrap_or(Value::Nil);
        let mut log = Vec::new();
        let value = if info.has_value {
            let (r, _, l) =
                self.call_resolved(a, hash, "value", Args::None, false, Some(info.clone()));
            log.extend(l);
            match r {
                Some(Ok(mut vals)) => from_lua(&vals.pop_front().unwrap_or(LuaValue::Nil), "value")
                    .unwrap_or_else(Value::Error),
                Some(Err(e)) => Value::Error(e),
                None => Value::Nil,
            }
        } else {
            raw
        };
        if let Value::Error(e) = &value
            && let Some(m) = &e.member
            && let Some(slot) = members.iter_mut().find(|(n, _)| **n == **m)
            && slot.1 == MemberStatus::Ok
        {
            slot.1 = MemberStatus::Runtime(e.clone());
        }
        let mut display = self.render(a, Some(hash), &value, Some(&info), Some(info.clone()));
        if let Some(e) = &display.display_error
            && let Some(slot) = members.iter_mut().find(|(n, _)| n == "display")
            && slot.1 == MemberStatus::Ok
        {
            slot.1 = MemberStatus::Runtime(e.clone());
        }
        if !value.is_error() {
            let (r, _, _) = self.call_resolved(
                a,
                hash,
                "validate",
                Args::Value(&value),
                false,
                Some(info.clone()),
            );
            if let Some(r) = r {
                display.warning = validation_message(r.map_err(Fault::Error)).map(Arc::from);
            }
        }
        Ok(Preview {
            members,
            value,
            display,
            log,
        })
    }
}

fn reject(message: impl Into<String>) -> EditRejected {
    EditRejected {
        message: message.into(),
    }
}

/// Interprets a `validate` result: `None` if valid, else the message.
fn validation_message(r: Result<MultiValue, Fault>) -> Option<String> {
    match r {
        Ok(mut vals) => {
            let ok = vals.pop_front().unwrap_or(LuaValue::Nil);
            let msg = vals.pop_front().unwrap_or(LuaValue::Nil);
            match ok {
                LuaValue::Boolean(false) | LuaValue::Nil => Some(match msg {
                    LuaValue::Nil => "invalid value".to_string(),
                    m => m.to_string().unwrap_or_default(),
                }),
                _ => None,
            }
        }
        Err(Fault::Error(e)) => Some(format!("validate failed: {}", e.message)),
        Err(Fault::NeedDep(_)) => None,
    }
}
