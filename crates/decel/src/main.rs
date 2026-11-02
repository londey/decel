// SPDX-License-Identifier: MIT OR Apache-2.0

//! decel: a spreadsheet where every cell can carry a Lua metatable.
//!
//! Usage: `decel [file.decel]`

mod actions;
mod client;
mod dock;
mod grid;
mod history;
mod palette;
mod recent;
mod sizes;
mod workspace;

use std::path::PathBuf;

use gpui::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, Theme, TitleBar};

use crate::workspace::Workspace;

fn main() {
    let open: Option<PathBuf> = std::env::args_os().nth(1).map(PathBuf::from);
    if open.as_deref().and_then(|p| p.to_str()) == Some("--version") {
        println!("decel {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    recent::prune_untitled();
    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);
        actions::bind_keys(cx);
        let (engine, events) = match client::start(decel_engine::Limits::default()) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("decel: cannot start the engine thread: {e}");
                cx.quit();
                return;
            }
        };
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitleBar::title_bar_options()),
                ..Default::default()
            },
            move |window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
                let view = cx.new(|cx| Workspace::new(engine, events, open, window, cx));
                window
                    .observe_window_appearance(|window, cx| {
                        Theme::sync_system_appearance(Some(window), cx);
                    })
                    .detach();
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
