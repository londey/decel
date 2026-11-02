// SPDX-License-Identifier: MIT OR Apache-2.0

//! decel: a spreadsheet where every cell can carry a Lua metatable.
//!
//! Usage: `decel [file.decel]`

mod actions;
mod charts;
mod client;
mod config;
mod csv_ui;
mod dock;
mod grid;
mod history;
mod palette;
mod palette_ui;
mod recent;
mod sheets;
mod sizes;
mod workspace;

use std::path::PathBuf;

use gpui::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, Theme, TitleBar};

use crate::workspace::Workspace;

/// Writes a crash log next to decel's data, then runs the default hook.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = recent::untitled_dir()
            .parent()
            .map(std::path::Path::to_path_buf);
        if let Some(dir) = dir {
            let _ = std::fs::create_dir_all(&dir);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let path = dir.join(format!("crash-{stamp}.log"));
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string();
            let text = format!(
                "decel {} crashed on thread {thread}\n{info}\n\n{}\n",
                env!("CARGO_PKG_VERSION"),
                std::backtrace::Backtrace::force_capture()
            );
            if std::fs::write(&path, text).is_ok() {
                eprintln!("decel: crash log written to {}", path.display());
            }
        }
        default(info);
    }));
}

fn main() {
    let open: Option<PathBuf> = std::env::args_os().nth(1).map(PathBuf::from);
    if open.as_deref().and_then(|p| p.to_str()) == Some("--version") {
        println!("decel {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    install_panic_hook();
    recent::prune_untitled();
    gpui_platform::application().run(move |cx: &mut App| {
        gpui_component::init(cx);
        let (config, mut config_error) = config::load();
        let bad = actions::bind_keys(cx, &config.keymap);
        if !bad.is_empty() && config_error.is_none() {
            config_error = Some(format!("could not bind {}", bad.join(", ")));
        }
        let (engine, events) = match client::start(config.limits()) {
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
                let view = cx.new(|cx| {
                    Workspace::new(engine, events, open, config, config_error, window, cx)
                });
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
