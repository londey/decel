// SPDX-License-Identifier: MIT OR Apache-2.0

//! The History dock (DESIGN §7.4, D10): the selected cell's versions with
//! member-level metatable diffs and restore, plus the undo history, where
//! clicking an entry undoes or redoes to that point.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use decel_engine::protocol::{Command, Reply};
use decel_engine::stdlib::fmt;
use decel_model::{Cell, CellAddr, MemberDiff, MetaHash, MetaSource, diff_members};
use decel_store::{CellVersion, TxnId, TxnInfo, TxnKind};
use gpui::{
    ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::ActiveTheme as _;

use crate::workspace::Workspace;

#[derive(Default)]
pub struct HistoryState {
    pub addr: Option<CellAddr>,
    pub versions: Vec<CellVersion>,
    pub selected: Option<TxnId>,
    pub metas: HashMap<MetaHash, MetaSource>,
    pub txns: HashMap<TxnId, TxnInfo>,
    pub undo: Vec<TxnId>,
    pub redo: Vec<TxnId>,
}

fn raw_text(c: &Cell) -> String {
    match &c.raw {
        Some(v) => v.edit_text(),
        None => "∅".into(),
    }
}

/// "just now", "5 min ago", "3 h ago", or a UTC date.
fn when(ts: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(ts);
    let secs = (now - ts).max(0) / 1000;
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => fmt::date(ts as f64 / 86_400_000.0, "%Y-%m-%d %H:%M"),
    }
}

/// The label of the action a stack entry stands for.
fn action_label(t: &TxnInfo) -> String {
    match t.kind {
        TxnKind::Redo => t
            .label
            .strip_prefix("Redo ")
            .unwrap_or(&t.label)
            .to_string(),
        TxnKind::Undo => t
            .label
            .strip_prefix("Undo ")
            .unwrap_or(&t.label)
            .to_string(),
        _ => t.label.clone(),
    }
}

impl Workspace {
    pub(crate) fn refresh_history(&mut self) {
        if !(self.dock.open && self.dock_tab == crate::workspace::DockTab::History) {
            return;
        }
        let Some(addr) = self.addr(self.sel.cursor.0, self.sel.cursor.1) else {
            return;
        };
        if self.history.addr != Some(addr) {
            self.history.selected = None;
        }
        self.history.addr = Some(addr);
        self.request(Command::QueryHistory { addr }, move |this, r, _, cx| {
            if this.history.addr == Some(addr)
                && let Ok(Reply::History(v)) = r
            {
                // Fetch metatable sources for diffs.
                let mut want: Vec<MetaHash> = v
                    .iter()
                    .flat_map(|x| [x.before.meta, x.after.meta])
                    .flatten()
                    .filter(|h| !this.history.metas.contains_key(h))
                    .collect();
                want.sort();
                want.dedup();
                for h in want {
                    this.request(Command::QueryMeta { hash: h }, move |this, r, _, cx| {
                        if let Ok(Reply::Meta(Some(src))) = r {
                            this.history.metas.insert(h, src);
                            cx.notify();
                        }
                    });
                }
                this.history.versions = v;
                cx.notify();
            }
        });
        self.request(Command::QueryTxns { limit: 500 }, |this, r, _, cx| {
            if let Ok(Reply::Txns { txns, undo, redo }) = r {
                this.history.txns = txns.into_iter().map(|t| (t.id, t)).collect();
                this.history.undo = undo;
                this.history.redo = redo;
                cx.notify();
            }
        });
    }

    /// Undoes or redoes until `steps` (negative: undo) have been taken.
    fn jump_history(&mut self, steps: i64) {
        let cmd = if steps < 0 {
            Command::Undo
        } else {
            Command::Redo
        };
        for _ in 0..steps.unsigned_abs() {
            self.send(cmd.clone());
        }
    }

    fn restore_version(&mut self, v: &CellVersion) {
        let Some(addr) = self.history.addr else {
            return;
        };
        self.send(Command::Restore {
            addr,
            version: v.txn.id,
            version_addr: v.addr,
        });
    }

    pub(crate) fn render_history(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let h = &self.history;
        let mut list = div()
            .id("history-scroll")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let title = match h.addr {
            Some(a) => format!("Cell {a}"),
            None => "No cell selected".into(),
        };
        list = list.child(
            div()
                .px_3()
                .py_1()
                .text_color(theme.muted_foreground)
                .child(format!("{title} · {} version(s)", h.versions.len())),
        );
        for v in &h.versions {
            let id = v.txn.id;
            let selected = h.selected == Some(id);
            let meta_changed = v.before.meta != v.after.meta;
            let row = div()
                .id(SharedString::from(format!("ver-{id}")))
                .px_3()
                .py_1()
                .flex()
                .flex_col()
                .cursor_pointer()
                .border_b_1()
                .border_color(theme.border.opacity(0.5))
                .when(selected, |d| d.bg(theme.secondary))
                .hover(|d| d.bg(theme.secondary))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(div().flex_1().child(v.txn.label.clone()))
                        .when(meta_changed, |d| {
                            d.child(
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .text_xs()
                                    .bg(theme.primary.opacity(0.2))
                                    .child("metatable"),
                            )
                        })
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(when(v.txn.ts)),
                        ),
                )
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .font_family(theme.mono_font_family.clone())
                        .child(format!("{} → {}", raw_text(&v.before), raw_text(&v.after))),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.history.selected = if this.history.selected == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                    cx.notify();
                }));
            list = list.child(row);
            if selected {
                list = list.child(self.render_version_detail(v, &theme, cx));
            }
        }

        // Undo history: done actions, the current position, undone actions.
        let mut undo = div()
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .max_h(px(260.))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_color(theme.muted_foreground)
                    .child("Undo history"),
            );
        let n_undo = h.undo.len();
        let start_steps = -(n_undo as i64);
        undo = undo.child(
            div()
                .id("hist-start")
                .px_3()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .hover(|d| d.bg(theme.secondary))
                .child("(opened workbook / start)")
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, _| this.jump_history(start_steps)),
                ),
        );
        let shown_from = n_undo.saturating_sub(12);
        for (i, id) in h.undo.iter().enumerate().skip(shown_from) {
            let label = h
                .txns
                .get(id)
                .map(action_label)
                .unwrap_or_else(|| format!("txn {id}"));
            let steps = -((n_undo - 1 - i) as i64);
            let current = i + 1 == n_undo;
            undo = undo.child(
                div()
                    .id(SharedString::from(format!("undo-{id}")))
                    .px_3()
                    .cursor_pointer()
                    .hover(|d| d.bg(theme.secondary))
                    .when(current, |d| d.border_l_2().border_color(theme.primary))
                    .child(label)
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, _| this.jump_history(steps)),
                    ),
            );
        }
        for (j, id) in h.redo.iter().enumerate().rev().take(12) {
            let label = h
                .txns
                .get(id)
                .map(action_label)
                .unwrap_or_else(|| format!("txn {id}"));
            let steps = (h.redo.len() - j) as i64;
            undo = undo.child(
                div()
                    .id(SharedString::from(format!("redo-{id}")))
                    .px_3()
                    .cursor_pointer()
                    .italic()
                    .text_color(theme.muted_foreground)
                    .hover(|d| d.bg(theme.secondary))
                    .child(label)
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, _| this.jump_history(steps)),
                    ),
            );
        }
        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_h(px(0.))
            .child(list)
            .child(undo)
    }

    fn render_version_detail(
        &self,
        v: &CellVersion,
        theme: &gpui_component::Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut detail = div()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .bg(theme.secondary.opacity(0.5));
        let version = v.clone();
        detail = detail.child(
            div()
                .id("restore")
                .px_2()
                .py_0p5()
                .rounded_sm()
                .bg(theme.secondary)
                .hover(|d| d.bg(theme.secondary_hover))
                .cursor_pointer()
                .child("Restore this version")
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, _| this.restore_version(&version)),
                ),
        );
        if v.before.meta != v.after.meta {
            let get = |h: Option<MetaHash>| -> Option<MetaSource> {
                match h {
                    None => Some(MetaSource::default()),
                    Some(h) => self.history.metas.get(&h).cloned(),
                }
            };
            match (get(v.before.meta), get(v.after.meta)) {
                (Some(b), Some(a)) => {
                    let mut same = 0;
                    for d in diff_members(&b, &a) {
                        let (tag, color, left, right) = match &d {
                            MemberDiff::Same(_) => {
                                same += 1;
                                continue;
                            }
                            MemberDiff::Added(m) => {
                                ("added", theme.success, None, Some(m.source.clone()))
                            }
                            MemberDiff::Removed(m) => {
                                ("removed", theme.danger, Some(m.source.clone()), None)
                            }
                            MemberDiff::Changed { before, after } => (
                                "changed",
                                theme.warning,
                                Some(before.source.clone()),
                                Some(after.source.clone()),
                            ),
                        };
                        let code = |t: Option<String>| {
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .p_1()
                                .rounded_sm()
                                .bg(theme.background)
                                .font_family(theme.mono_font_family.clone())
                                .text_xs()
                                .children(
                                    t.unwrap_or_default()
                                        .lines()
                                        .map(|l| div().child(l.to_string()))
                                        .collect::<Vec<_>>(),
                                )
                        };
                        detail = detail
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .child(div().child(d.name().to_string()))
                                    .child(div().text_color(color).child(tag)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_1()
                                    .child(code(left))
                                    .child(code(right)),
                            );
                    }
                    if same > 0 {
                        detail = detail.child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(format!("{same} unchanged member(s)")),
                        );
                    }
                }
                _ => detail = detail.child("Loading metatables…"),
            }
        }
        detail
    }
}
