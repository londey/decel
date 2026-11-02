// SPDX-License-Identifier: MIT OR Apache-2.0

//! The Metatable dock (DESIGN §8.4): a REALbasic-style member list with one
//! member edited at a time, a live preview against the selected cell, and
//! copy-on-write or shared commits (§5.3).
//!
//! Uncommitted text is kept per member in `drafts` until it is committed
//! (`Ctrl+Enter`, the Commit button, or moving the dock to another cell) or
//! reverted (`Esc` reverts the selected member).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use decel_engine::protocol::{Command, MetaEditMode, Reply};
use decel_engine::{MemberStatus, Preview};
use decel_model::{CellAddr, FIELDS, HOOKS, Member, MemberKind, MetaHash, MetaSource};
use gpui::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Task, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::ActiveTheme as _;
use gpui_component::input::{Editor, EditorState, InputEvent};

use crate::workspace::Workspace;

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(150);
pub const DOCK_W: f32 = 460.0;

pub struct Dock {
    pub open: bool,
    pub target: Option<CellAddr>,
    pub mode: MetaEditMode,
    /// The committed metatable of the target (`None`: the cell has none).
    pub source: Option<MetaSource>,
    pub hash: Option<MetaHash>,
    /// `(cells sharing the metatable, how many are on other sheets)`.
    pub share: Option<(usize, usize)>,
    /// Edited or added members, by name.
    pub drafts: BTreeMap<String, (MemberKind, String)>,
    pub removed: BTreeSet<String>,
    pub selected: Option<String>,
    pub editor: Entity<EditorState>,
    /// Set while the editor's text is replaced programmatically.
    loading: bool,
    pub preview: Option<Preview>,
    pub adding: bool,
    _sub: Option<Subscription>,
    _preview_task: Option<Task<()>>,
}

impl Dock {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("lua")
                .line_number(true)
        });
        Self {
            open: false,
            target: None,
            mode: MetaEditMode::Cow,
            source: None,
            hash: None,
            share: None,
            drafts: BTreeMap::new(),
            removed: BTreeSet::new(),
            selected: None,
            editor,
            loading: false,
            preview: None,
            adding: false,
            _sub: None,
            _preview_task: None,
        }
    }

    pub fn dirty(&self) -> bool {
        !self.drafts.is_empty() || !self.removed.is_empty()
    }

    /// The committed source with drafts applied.
    pub fn working(&self) -> MetaSource {
        let mut src = self.source.clone().unwrap_or_default();
        for name in &self.removed {
            src.remove(name);
        }
        for (name, (kind, text)) in &self.drafts {
            src.set(Member {
                name: name.clone(),
                kind: *kind,
                source: text.clone(),
            });
        }
        src
    }

    /// Members in display order: fields, standard hooks, then helpers.
    pub fn members(&self) -> Vec<Member> {
        let src = self.working();
        let mut out: Vec<Member> = Vec::new();
        for f in FIELDS {
            if let Some(m) = src.get(f) {
                out.push(m.clone());
            }
        }
        for (h, _) in HOOKS {
            if let Some(m) = src.get(h) {
                out.push(m.clone());
            }
        }
        let mut helpers: Vec<Member> = src
            .members
            .iter()
            .filter(|m| !FIELDS.contains(&m.name.as_str()) && m.signature().is_none())
            .cloned()
            .collect();
        helpers.sort_by(|a, b| a.name.cmp(&b.name));
        out.extend(helpers);
        out
    }

    fn status(&self, name: &str) -> Option<&MemberStatus> {
        self.preview
            .as_ref()?
            .members
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s)
    }
}

impl Workspace {
    /// Wires the dock editor's change events (called once after creation).
    pub(crate) fn init_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.dock.editor.clone();
        let sub = cx.subscribe_in(&editor, window, |this, editor, e: &InputEvent, _, cx| {
            if !matches!(e, InputEvent::Change) || this.dock.loading {
                return;
            }
            let Some(name) = this.dock.selected.clone() else {
                return;
            };
            let text = editor.read(cx).value().to_string();
            let kind = this
                .dock
                .members()
                .iter()
                .find(|m| m.name == name)
                .map_or(MemberKind::for_name(&name), |m| m.kind);
            let committed = this
                .dock
                .source
                .as_ref()
                .and_then(|s| s.get(&name))
                .map(|m| m.source.clone());
            if committed.as_deref() == Some(text.as_str()) {
                this.dock.drafts.remove(&name);
            } else {
                this.dock.drafts.insert(name, (kind, text));
            }
            this.schedule_preview(cx);
            cx.notify();
        });
        self.dock._sub = Some(sub);
    }

    pub(crate) fn toggle_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.open = !self.dock.open;
        if self.dock.open {
            self.dock_follow_selection(window, cx);
        }
        cx.notify();
    }

    /// Points the dock at the selected cell, committing pending drafts for the
    /// previous cell first.
    pub(crate) fn dock_follow_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(addr) = self.addr(self.sel.cursor.0, self.sel.cursor.1) else {
            return;
        };
        if self.dock.target == Some(addr) && self.dock.source.is_some() {
            return;
        }
        self.dock_target(addr, MetaEditMode::Cow, window, cx);
    }

    pub(crate) fn dock_target(
        &mut self,
        addr: CellAddr,
        mode: MetaEditMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dock.dirty() && self.dock.target != Some(addr) {
            self.commit_meta(window, cx);
        }
        self.dock.open = true;
        self.dock.target = Some(addr);
        self.dock.mode = mode;
        self.dock.drafts.clear();
        self.dock.removed.clear();
        self.dock.preview = None;
        self.dock.adding = false;
        let hash = self.cache.get(&addr).and_then(|d| d.hash);
        self.dock.hash = hash;
        self.dock.share = None;
        match hash {
            None => {
                self.dock.source = None;
                self.dock.selected = None;
                self.load_member_text(window, cx);
            }
            Some(h) => {
                self.request(
                    Command::QueryMeta { hash: h },
                    move |this, r, window, cx| {
                        if this.dock.hash != Some(h) {
                            return;
                        }
                        if let Ok(Reply::Meta(src)) = r {
                            this.dock.source = src;
                            let first = this.dock.members().first().map(|m| m.name.clone());
                            this.dock.selected = this
                                .dock
                                .selected
                                .clone()
                                .filter(|s| this.dock.working().get(s).is_some())
                                .or(first);
                            this.load_member_text(window, cx);
                            this.schedule_preview(cx);
                        }
                        cx.notify();
                    },
                );
                self.request(Command::QueryShareSet { hash: h }, move |this, r, _, cx| {
                    if this.dock.hash == Some(h)
                        && let Ok(Reply::ShareSet(cells)) = r
                    {
                        let other = cells.iter().filter(|a| a.sheet != addr.sheet).count();
                        this.dock.share = Some((cells.len(), other));
                        cx.notify();
                    }
                });
            }
        }
        cx.notify();
    }

    /// Loads the selected member's text (draft or committed) into the editor.
    fn load_member_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .dock
            .selected
            .as_ref()
            .and_then(|n| self.dock.working().get(n).map(|m| m.source.clone()))
            .unwrap_or_default();
        self.dock.loading = true;
        self.dock
            .editor
            .update(cx, |e, cx| e.set_value(text, window, cx));
        self.dock.loading = false;
    }

    fn select_member(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.selected = Some(name);
        self.dock.adding = false;
        self.load_member_text(window, cx);
        self.dock.editor.update(cx, |e, cx| e.focus(window, cx));
        cx.notify();
    }

    fn add_member(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let kind = MemberKind::for_name(&name);
        let text = match kind {
            MemberKind::Hook => match name.as_str() {
                "value" => "return c.raw".to_string(),
                "display" => "return tostring(v)".to_string(),
                "edit" => "return tostring(v)".to_string(),
                "parse" => "return tonumber(text) or text".to_string(),
                "validate" => "return true".to_string(),
                "tooltip" | "edit_hint" => "return nil".to_string(),
                _ => String::new(),
            },
            MemberKind::Field => "\"\"".to_string(),
            MemberKind::Helper => "function(self)\n  return nil\nend".to_string(),
        };
        self.dock.removed.remove(&name);
        self.dock.drafts.insert(name.clone(), (kind, text));
        self.select_member(name, window, cx);
        self.schedule_preview(cx);
    }

    fn remove_member(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.drafts.remove(name);
        if self
            .dock
            .source
            .as_ref()
            .is_some_and(|s| s.get(name).is_some())
        {
            self.dock.removed.insert(name.to_string());
        }
        if self.dock.selected.as_deref() == Some(name) {
            self.dock.selected = self.dock.members().first().map(|m| m.name.clone());
            self.load_member_text(window, cx);
        }
        self.schedule_preview(cx);
        cx.notify();
    }

    /// Reverts the selected member to its committed text.
    pub(crate) fn revert_member(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = self.dock.selected.clone() {
            self.dock.drafts.remove(&name);
            self.dock.removed.remove(&name);
            if self.dock.working().get(&name).is_none() {
                self.dock.selected = self.dock.members().first().map(|m| m.name.clone());
            }
            self.load_member_text(window, cx);
            self.schedule_preview(cx);
            cx.notify();
        }
    }

    fn schedule_preview(&mut self, cx: &mut Context<Self>) {
        let Some(addr) = self.dock.target else { return };
        self.dock._preview_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
            let _ = this.update(cx, |this, _| {
                if this.dock.target != Some(addr) {
                    return;
                }
                let source = this.dock.working();
                this.request(Command::Preview { addr, source }, move |this, r, _, cx| {
                    if this.dock.target == Some(addr)
                        && let Ok(Reply::Preview(p)) = r
                    {
                        this.dock.preview = Some(*p);
                        cx.notify();
                    }
                });
            });
        }));
    }

    /// Commits the working metatable (copy-on-write or shared).
    pub(crate) fn commit_meta(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.dock.dirty() {
            return;
        }
        let Some(addr) = self.dock.target else { return };
        let source = self.dock.working();
        let mode = self.dock.mode;
        let hash = source.hash();
        self.dock.source = Some(source.clone());
        self.dock.hash = Some(hash);
        self.dock.drafts.clear();
        self.dock.removed.clear();
        self.dock.mode = MetaEditMode::Cow;
        self.request(
            Command::SetMeta {
                targets: vec![addr],
                source: Some(source),
                mode,
            },
            move |this, r, _, cx| {
                if let Err(e) = r {
                    this.banner = Some(crate::workspace::Banner::Error(e.to_string()));
                }
                if this.dock.target == Some(addr) {
                    this.request(Command::QueryShareSet { hash }, move |this, r, _, cx| {
                        if this.dock.hash == Some(hash)
                            && let Ok(Reply::ShareSet(cells)) = r
                        {
                            let other = cells.iter().filter(|a| a.sheet != addr.sheet).count();
                            this.dock.share = Some((cells.len(), other));
                            cx.notify();
                        }
                    });
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn detach_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(addr) = self.dock.target else { return };
        self.commit_meta(window, cx);
        self.request(
            Command::Detach {
                targets: vec![addr],
            },
            move |this, _, window, cx| {
                // Reload once the cell's new hash has arrived.
                this.dock.source = None;
                this.dock.target = None;
                if this.addr(this.sel.cursor.0, this.sel.cursor.1) == Some(addr) {
                    cx.defer_in(window, move |this, window, cx| {
                        this.dock_target(addr, MetaEditMode::Cow, window, cx)
                    });
                }
            },
        );
    }

    pub(crate) fn new_meta_on_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet else { return };
        let rect = self.sel.rect();
        if rect.rows() * rect.cols() > 1_000_000 {
            return;
        }
        let targets: Vec<CellAddr> = rect
            .iter()
            .map(|(r, c)| CellAddr::new(sheet, r, c))
            .collect();
        let addr = self.addr(self.sel.cursor.0, self.sel.cursor.1);
        self.request(Command::NewMeta { targets }, move |this, _, window, cx| {
            if let Some(addr) = addr {
                cx.defer_in(window, move |this, window, cx| {
                    this.dock_target(addr, MetaEditMode::Cow, window, cx)
                });
            }
            let _ = this;
        });
        self.dock.open = true;
        let _ = window;
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Rendering

    pub(crate) fn render_dock(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.dock.open {
            return None;
        }
        let theme = cx.theme().clone();
        let dock = &self.dock;
        let tab = |id: &'static str, label: &'static str, active: bool| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .cursor_pointer()
                .when(active, |d| d.border_b_2().border_color(theme.primary))
                .when(!active, |d| d.text_color(theme.muted_foreground))
                .child(label)
        };
        let on_meta = self.dock_tab == crate::workspace::DockTab::Metatable;
        let mut panel = div()
            .w(px(DOCK_W))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        tab("tab-meta", "Metatable  F4", on_meta).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.dock_tab = crate::workspace::DockTab::Metatable;
                                this.dock_follow_selection(window, cx);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        tab("tab-history", "History  F5", !on_meta).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.dock_tab = crate::workspace::DockTab::History;
                                this.history.addr = None;
                                this.refresh_history();
                                cx.notify();
                            },
                        )),
                    ),
            );
        if !on_meta {
            return Some(panel.child(self.render_history(cx)));
        }
        let Some(addr) = dock.target else {
            return Some(panel.child(div().p_3().child("Select a cell.")));
        };
        if dock.source.is_none() && dock.drafts.is_empty() {
            return Some(
                panel.child(
                    div()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(format!("Cell {addr} has no metatable."))
                        .child(
                            button(
                                "new-meta",
                                "New metatable on selection   Ctrl+Shift+M",
                                &theme,
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    this.new_meta_on_selection(window, cx)
                                },
                            )),
                        ),
                ),
            );
        }

        // COW / shared banner.
        let banner = match (dock.mode, dock.share) {
            (MetaEditMode::Shared, Some((n, other))) => {
                format!("Editing {n} cells ({other} on other sheets)")
            }
            (MetaEditMode::Shared, None) => "Editing every cell that shares this metatable".into(),
            (MetaEditMode::Cow, Some((n, _))) if n > 1 => {
                format!("Editing this cell only · shared by {} others", n - 1)
            }
            (MetaEditMode::Cow, _) => "Editing this cell only".into(),
        };
        let shared = dock.mode == MetaEditMode::Shared;
        panel = panel.child(
            div()
                .px_3()
                .py_1()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .when(shared, |d| d.bg(theme.warning.opacity(0.2)))
                .child(div().flex_1().child(banner))
                .when(dock.share.is_some_and(|(n, _)| n > 1), |d| {
                    d.child(button("detach", "Detach", &theme).on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| this.detach_target(window, cx),
                    )))
                }),
        );

        // Member list | editor.
        let members = dock.members();
        let mut list = div()
            .w(px(150.))
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(theme.border)
            .py_1();
        for m in &members {
            let name = m.name.clone();
            let selected = dock.selected.as_deref() == Some(m.name.as_str());
            let (icon, color) = match dock.status(&m.name) {
                Some(MemberStatus::Syntax(_)) => ("✗", theme.danger),
                Some(MemberStatus::Runtime(_)) => ("⚠", theme.warning),
                Some(MemberStatus::Ok) => ("✓", theme.success),
                None => (" ", theme.muted_foreground),
            };
            let drafted = dock.drafts.contains_key(&m.name);
            let rm = m.name.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("member-{}", m.name)))
                    .px_2()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.secondary_active))
                    .hover(|d| d.bg(theme.secondary))
                    .child(div().w(px(12.)).text_color(color).child(icon))
                    .child(div().flex_1().child(format!(
                        "{}{}",
                        m.name,
                        if drafted { " •" } else { "" }
                    )))
                    .when(selected, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("rm-{}", m.name)))
                                .text_color(theme.muted_foreground)
                                .child("✕")
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.remove_member(&rm, window, cx)
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.select_member(name.clone(), window, cx)
                    })),
            );
        }
        list = list
            .child(
                div()
                    .id("add-hook")
                    .px_2()
                    .pt_2()
                    .cursor_pointer()
                    .text_color(theme.muted_foreground)
                    .child("+ add hook")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.dock.adding = !this.dock.adding;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("add-helper")
                    .px_2()
                    .cursor_pointer()
                    .text_color(theme.muted_foreground)
                    .child("+ helper")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        let names: Vec<String> =
                            this.dock.members().iter().map(|m| m.name.clone()).collect();
                        let mut n = 1;
                        while names.contains(&format!("helper{n}")) {
                            n += 1;
                        }
                        this.add_member(format!("helper{n}"), window, cx)
                    })),
            );
        if dock.adding {
            let present: Vec<String> = members.iter().map(|m| m.name.clone()).collect();
            for name in HOOKS
                .iter()
                .map(|(h, _)| *h)
                .chain(FIELDS.iter().copied())
                .filter(|n| !present.iter().any(|p| p == n))
            {
                let n = name.to_string();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("add-{name}")))
                        .pl_4()
                        .cursor_pointer()
                        .hover(|d| d.bg(theme.secondary))
                        .child(name)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.add_member(n.clone(), window, cx)
                        })),
                );
            }
        }

        let selected = dock
            .selected
            .as_ref()
            .and_then(|n| members.iter().find(|m| &m.name == n));
        let (header, footer) = match selected {
            Some(m) => match m.signature() {
                Some(params) => (format!("function {}({params})", m.name), Some("end")),
                None => (format!("{} =", m.name), None),
            },
            None => (String::new(), None),
        };
        let editor_col = div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(header),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(120.))
                    .child(Editor::new(&dock.editor).h_full().bordered(false)),
            )
            .when_some(footer, |d, f| {
                d.child(
                    div()
                        .px_2()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .child(f),
                )
            });
        panel = panel.child(
            div()
                .flex_1()
                .flex()
                .flex_row()
                .border_t_1()
                .border_color(theme.border)
                .child(list)
                .child(editor_col),
        );

        // Status, preview and log.
        let mut status = div()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .border_t_1()
            .border_color(theme.border);
        if let Some(p) = &dock.preview {
            let errors: Vec<String> = p
                .members
                .iter()
                .filter_map(|(n, s)| match s {
                    MemberStatus::Syntax(e) | MemberStatus::Runtime(e) => Some(format!(
                        "{n}{}: {}",
                        e.line.map(|l| format!(":{l}")).unwrap_or_default(),
                        e.message
                    )),
                    MemberStatus::Ok => None,
                })
                .collect();
            if errors.is_empty() {
                status = status.child(div().text_color(theme.success).child("✓ compiles"));
            }
            for e in errors {
                status = status.child(div().text_color(theme.danger).child(e));
            }
            status = status.child(format!("preview: {}", p.display.text));
            if let Some(w) = &p.display.warning {
                status = status.child(div().text_color(theme.warning).child(format!("⚠ {w}")));
            }
            for line in p.log.iter().take(20) {
                status = status.child(
                    div()
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .child(format!("log: {line}")),
                );
            }
        }
        status = status.child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .pt_1()
                .child(
                    button(
                        "commit",
                        if shared {
                            "Commit to all   Ctrl+Enter"
                        } else {
                            "Commit   Ctrl+Enter"
                        },
                        &theme,
                    )
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.commit_meta(window, cx)
                        }),
                    ),
                )
                .child(button("revert", "Revert   Esc", &theme).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.revert_member(window, cx)),
                )),
        );
        Some(panel.child(status))
    }
}

fn button(
    id: &'static str,
    label: &'static str,
    theme: &gpui_component::Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_2()
        .py_0p5()
        .rounded_sm()
        .bg(theme.secondary)
        .hover(|d| d.bg(theme.secondary_hover))
        .cursor_pointer()
        .child(label)
}
