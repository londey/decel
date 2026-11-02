// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sheet tabs (add, inline rename, drag to reorder, delete with
//! confirmation) and row/column insertion and deletion (DESIGN §8.1, Phase 7).
//! Every change is an undoable engine transaction.

use decel_engine::protocol::Command;
use decel_model::{SheetId, StructOp};
use gpui::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::ActiveTheme as _;
use gpui_component::input::{Input, InputState, Position};

use crate::workspace::{SelMode, Workspace};

/// A tab being dragged to a new position.
#[derive(Clone, Copy)]
pub struct TabDrag(pub SheetId);

/// What follows the pointer while a tab is dragged.
pub struct TabDragPreview(pub SharedString);

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .rounded_sm()
            .shadow_md()
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().primary)
            .child(self.0.clone())
    }
}

pub struct Renaming {
    pub id: SheetId,
    pub input: Entity<InputState>,
}

impl Workspace {
    fn sheet_names(&self) -> Vec<String> {
        self.info
            .as_ref()
            .map(|i| i.sheets.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default()
    }

    pub(crate) fn add_sheet(&mut self, cx: &mut Context<Self>) {
        let names = self.sheet_names();
        let mut n = names.len() + 1;
        while names.contains(&format!("Sheet{n}")) {
            n += 1;
        }
        let name = format!("Sheet{n}");
        self.switch_to_name = Some(name.clone());
        self.send(Command::AddSheet { name });
        cx.notify();
    }

    fn start_rename(&mut self, id: SheetId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .info
            .as_ref()
            .and_then(|i| i.sheets.iter().find(|s| s.id == id))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let len = name.encode_utf16().count() as u32;
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(name, window, cx);
            s.set_cursor_position(Position::new(0, len), window, cx);
            s
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.renaming = Some(Renaming { id, input });
        cx.notify();
    }

    /// Enter commits the rename, Escape cancels. Returns whether consumed.
    pub(crate) fn on_rename_key(
        &mut self,
        k: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match k.key.as_str() {
            "enter" => {
                if let Some(r) = self.renaming.take() {
                    let name = r.input.read(cx).value().trim().to_string();
                    if !name.is_empty() {
                        self.send(Command::RenameSheet { id: r.id, name });
                    }
                }
                self.focus.focus(window, cx);
                cx.notify();
                true
            }
            "escape" => {
                self.renaming = None;
                self.focus.focus(window, cx);
                cx.notify();
                true
            }
            _ => false,
        }
    }

    fn move_sheet(&mut self, id: SheetId, to: SheetId) {
        let Some(info) = &self.info else { return };
        if let Some(position) = info.sheets.iter().position(|s| s.id == to) {
            self.send(Command::MoveSheet {
                id,
                position: position as u32,
            });
        }
    }

    fn delete_sheet(&mut self, id: SheetId, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        self.send(Command::DeleteSheet { id });
        cx.notify();
    }

    /// Insert (`delete == false`) or delete rows/columns at the selection.
    /// Whole-column selections act on columns; anything else on rows.
    pub(crate) fn structure_at_selection(&mut self, delete: bool, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet else { return };
        let r = self.sel.rect();
        let op = if self.sel.mode == SelMode::Cols {
            let (at, n) = (r.c1, r.c2.saturating_sub(r.c1).saturating_add(1));
            if delete {
                StructOp::DeleteCols { sheet, at, n }
            } else {
                StructOp::InsertCols { sheet, at, n }
            }
        } else {
            let (at, n) = (r.r1, r.r2.saturating_sub(r.r1).saturating_add(1));
            if delete {
                StructOp::DeleteRows { sheet, at, n }
            } else {
                StructOp::InsertRows { sheet, at, n }
            }
        };
        if op.span().1 > 100_000 {
            return;
        }
        self.send(Command::Structure(op));
        cx.notify();
    }

    pub(crate) fn render_sheet_tabs(&self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = cx.theme().clone();
        let mut row = div().flex().flex_row().items_center().h_full();
        let Some(info) = &self.info else { return row };
        let only_one = info.sheets.len() <= 1;
        for s in &info.sheets {
            let id = s.id;
            let active = Some(id) == self.sheet;
            let renaming = self.renaming.as_ref().filter(|r| r.id == id);
            let name: SharedString = s.name.clone().into();
            let mut tab = div()
                .id(SharedString::from(format!("tab-{}", id.0)))
                .px_3()
                .h_full()
                .flex()
                .flex_row()
                .gap_2()
                .items_center()
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(theme.background)
                        .border_b_2()
                        .border_color(theme.primary)
                })
                .drag_over::<TabDrag>(|d, _, _, cx| d.bg(cx.theme().primary.opacity(0.2)))
                .on_drop(cx.listener(move |this, d: &TabDrag, _, cx| {
                    this.move_sheet(d.0, id);
                    cx.notify();
                }));
            tab = match renaming {
                Some(r) => tab.child(
                    div()
                        .w(px(120.))
                        .child(Input::new(&r.input).appearance(false)),
                ),
                None => {
                    let preview_name = name.clone();
                    tab.child(name.clone())
                        .on_drag(TabDrag(id), move |_, _, _, cx| {
                            cx.new(|_| TabDragPreview(preview_name.clone()))
                        })
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if e.click_count() >= 2 {
                                this.start_rename(id, window, cx);
                            } else {
                                this.switch_sheet(id, window, cx);
                            }
                        }))
                }
            };
            if active && !only_one && renaming.is_none() {
                tab = tab.child(
                    div()
                        .id(SharedString::from(format!("del-{}", id.0)))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .hover(|d| d.text_color(theme.danger))
                        .child("✕")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.confirm_delete = Some(id);
                            cx.notify();
                        })),
                );
            }
            row = row.child(tab);
        }
        row = row.child(
            div()
                .id("add-sheet")
                .px_3()
                .h_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .hover(|d| d.bg(theme.secondary_hover))
                .child("+")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_sheet(cx))),
        );
        if let Some(id) = self.confirm_delete
            && let Some(s) = info.sheets.iter().find(|s| s.id == id)
        {
            row = row.child(
                div()
                    .ml_3()
                    .px_2()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .bg(theme.danger.opacity(0.15))
                    .child(format!(
                        "Delete sheet “{}”? (Undo can bring it back.)",
                        s.name
                    ))
                    .child(
                        div()
                            .id("confirm-del")
                            .px_2()
                            .rounded_sm()
                            .bg(theme.danger)
                            .text_color(theme.danger_foreground)
                            .cursor_pointer()
                            .child("Delete")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.delete_sheet(id, cx)
                            })),
                    )
                    .child(
                        div()
                            .id("cancel-del")
                            .px_2()
                            .cursor_pointer()
                            .child("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.confirm_delete = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        row
    }
}
