// SPDX-License-Identifier: MIT OR Apache-2.0

//! decel desktop application.
//!
//! Phase 0 spike: a window with a custom-painted dummy grid and a Lua code
//! editor, proving the GPUI toolchain on Linux and Windows.

mod spike_grid;

use gpui::{App, AppContext as _, Bounds, Context, Entity, Window, WindowBounds, WindowOptions};
use gpui::{IntoElement, ParentElement as _, Render, Styled as _, div, px, size};
use gpui_component::input::{Editor, EditorState};
use gpui_component::{ActiveTheme as _, Root};

use crate::spike_grid::GridView;

const SAMPLE_LUA: &str = r#"-- Latitude stored as decimal degrees, shown as D°M′S″.
return {
  kind = "latitude",
  unit = "deg",

  display = function(c, v)
    if type(v) ~= "number" then return tostring(v) end
    local h = v < 0 and "S" or "N"
    return fmt.dms(math.abs(v)) .. " " .. h
  end,

  validate = function(c, v)
    if v < -90 or v > 90 then
      return false, "latitude must be within ±90°"
    end
    return true
  end,
}
"#;

struct Spike {
    grid: Entity<GridView>,
    editor: Entity<EditorState>,
}

impl Spike {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let grid = cx.new(|_| GridView::new(1_000, 50));
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("lua")
                .line_number(true)
                .default_value(SAMPLE_LUA)
        });
        Self { grid, editor }
    }
}

impl Render for Spike {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().flex_1().h_full().child(self.grid.clone()))
            .child(
                div()
                    .w(px(480.))
                    .h_full()
                    .p_2()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(Editor::new(&self.editor).h_full()),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        gpui_component::init(cx);
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| Spike::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        if let Err(err) = opened {
            eprintln!("decel: failed to open window: {err:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}
