// SPDX-License-Identifier: MIT OR Apache-2.0

//! The command palette (`Ctrl+Shift+P`): fuzzy search over every command
//! and recent file, showing key bindings (DESIGN §8.1).

use std::path::PathBuf;

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::ActiveTheme as _;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::actions::commands;
use crate::workspace::Workspace;

const MAX_ITEMS: usize = 14;

pub struct Palette {
    pub input: Entity<InputState>,
    pub selected: usize,
    _sub: Subscription,
}

#[derive(Clone)]
pub enum Item {
    /// Index into [`commands`].
    Command(usize),
    Recent(PathBuf),
}

/// Subsequence fuzzy match. Higher is better; `None` if `query` isn't a
/// subsequence of `text` (case-insensitive).
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut prev: Option<usize> = None;
    for (ti, ch) in t.iter().enumerate() {
        if qi < q.len() && *ch == q[qi] {
            score += 10;
            if prev == Some(ti.wrapping_sub(1)) {
                score += 15;
            }
            if ti == 0 || !t[ti - 1].is_alphanumeric() {
                score += 20;
            }
            if let Some(p) = prev {
                score -= (ti - p - 1).min(10) as i32;
            }
            prev = Some(ti);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score - t.len() as i32 / 8)
}

/// `ctrl-shift-p` → `Ctrl+Shift+P`.
pub fn key_label(keys: &str) -> String {
    keys.split(' ')
        .map(|stroke| {
            // A trailing "--" means the minus key.
            if let Some(mods) = stroke.strip_suffix("--") {
                let m = key_label(mods);
                return if m.is_empty() {
                    "-".into()
                } else {
                    format!("{m}+-")
                };
            }
            stroke
                .split('-')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let mut c = p.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Workspace {
    pub(crate) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a command…"));
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::Change)
                && let Some(p) = this.palette.as_mut()
            {
                p.selected = 0;
                cx.notify();
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.palette = Some(Palette {
            input,
            selected: 0,
            _sub: sub,
        });
        cx.notify();
    }

    pub(crate) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn palette_items(&self, cx: &Context<Self>) -> Vec<(Item, String, String)> {
        let Some(p) = &self.palette else {
            return Vec::new();
        };
        let query = p.input.read(cx).value().to_string();
        let mut scored: Vec<(i32, Item, String, String)> = Vec::new();
        for (i, c) in commands().iter().enumerate() {
            if !c.palette {
                continue;
            }
            if let Some(s) = fuzzy(&query, c.title) {
                let keys = self
                    .config
                    .keymap
                    .get(c.name)
                    .map(String::as_str)
                    .or(c.keys.first().copied())
                    .map(key_label)
                    .unwrap_or_default();
                scored.push((s, Item::Command(i), c.title.to_string(), keys));
            }
        }
        for p in crate::recent::load() {
            let title = format!("Open recent: {}", p.display());
            if !query.is_empty()
                && let Some(s) = fuzzy(&query, &title)
            {
                scored.push((s - 5, Item::Recent(p), title, String::new()));
            }
        }
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored
            .into_iter()
            .take(MAX_ITEMS)
            .map(|(_, i, t, k)| (i, t, k))
            .collect()
    }

    fn run_palette_item(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
        match item {
            Item::Command(i) => {
                if let Some(c) = commands().into_iter().nth(i) {
                    window.dispatch_action(c.action, cx);
                }
            }
            Item::Recent(p) => self.open_path(p, false, window, cx),
        }
    }

    /// Up/Down move, Enter runs, Escape closes. Returns whether consumed.
    pub(crate) fn on_palette_key(
        &mut self,
        k: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let n = self.palette_items(cx).len();
        let Some(p) = self.palette.as_mut() else {
            return false;
        };
        match k.key.as_str() {
            "down" => {
                p.selected = (p.selected + 1).min(n.saturating_sub(1));
                cx.notify();
                true
            }
            "up" => {
                p.selected = p.selected.saturating_sub(1);
                cx.notify();
                true
            }
            "enter" => {
                let sel = p.selected;
                if let Some((item, _, _)) = self.palette_items(cx).into_iter().nth(sel) {
                    self.run_palette_item(item, window, cx);
                }
                true
            }
            "escape" => {
                self.close_palette(window, cx);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn render_palette(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let p = self.palette.as_ref()?;
        let theme = cx.theme().clone();
        let items = self.palette_items(cx);
        let mut list = div().flex().flex_col().py_1();
        for (i, (item, title, keys)) in items.into_iter().enumerate() {
            let selected = i == p.selected;
            list = list.child(
                div()
                    .id(SharedString::from(format!("palette-{i}")))
                    .px_3()
                    .py_1()
                    .flex()
                    .flex_row()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.secondary_active))
                    .hover(|d| d.bg(theme.secondary))
                    .child(div().flex_1().overflow_hidden().child(title))
                    .child(div().text_color(theme.muted_foreground).child(keys))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.run_palette_item(item.clone(), window, cx)
                    })),
            );
        }
        Some(
            div()
                .id("palette")
                .occlude()
                .absolute()
                .top(px(40.))
                .left_1_4()
                .w_1_2()
                .rounded_md()
                .shadow_lg()
                .bg(theme.popover)
                .border_1()
                .border_color(theme.border)
                .child(div().p_2().child(Input::new(&p.input)))
                .child(list),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_word_starts_and_runs() {
        assert!(fuzzy("fd", "Fill down").is_some());
        assert!(fuzzy("xyz", "Fill down").is_none());
        let a = fuzzy("fill", "Fill down").unwrap();
        let b = fuzzy("fill", "Auto-fill column").unwrap();
        assert!(a > b);
        let c = fuzzy("ad", "Add sheet").unwrap();
        let d = fuzzy("ad", "Auto-fill column…").unwrap_or(i32::MIN);
        assert!(c > d);
        assert_eq!(fuzzy("", "anything"), Some(0));
    }

    #[test]
    fn key_labels() {
        assert_eq!(key_label("ctrl-shift-p"), "Ctrl+Shift+P");
        assert_eq!(key_label("f3"), "F3");
        assert_eq!(key_label("ctrl--"), "Ctrl+-");
    }
}
