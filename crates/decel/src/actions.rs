// SPDX-License-Identifier: MIT OR Apache-2.0

//! Actions and the default keymap (DESIGN §8.7).

use gpui::{App, KeyBinding, actions};

actions!(
    decel,
    [
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        SelectUp,
        SelectDown,
        SelectLeft,
        SelectRight,
        JumpUp,
        JumpDown,
        JumpLeft,
        JumpRight,
        PageUp,
        PageDown,
        RowStart,
        RowEnd,
        SheetStart,
        EditCell,
        ClearCells,
        Undo,
        Redo,
        NewFile,
        OpenFile,
        SaveAs,
        Quit,
    ]
);

/// The grid's key context.
pub const GRID: &str = "Grid";

pub fn bind_keys(cx: &mut App) {
    let g = Some(GRID);
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, g),
        KeyBinding::new("down", MoveDown, g),
        KeyBinding::new("left", MoveLeft, g),
        KeyBinding::new("right", MoveRight, g),
        KeyBinding::new("shift-up", SelectUp, g),
        KeyBinding::new("shift-down", SelectDown, g),
        KeyBinding::new("shift-left", SelectLeft, g),
        KeyBinding::new("shift-right", SelectRight, g),
        KeyBinding::new("ctrl-up", JumpUp, g),
        KeyBinding::new("ctrl-down", JumpDown, g),
        KeyBinding::new("ctrl-left", JumpLeft, g),
        KeyBinding::new("ctrl-right", JumpRight, g),
        KeyBinding::new("pageup", PageUp, g),
        KeyBinding::new("pagedown", PageDown, g),
        KeyBinding::new("home", RowStart, g),
        KeyBinding::new("end", RowEnd, g),
        KeyBinding::new("ctrl-home", SheetStart, g),
        KeyBinding::new("f2", EditCell, g),
        KeyBinding::new("enter", EditCell, g),
        KeyBinding::new("delete", ClearCells, g),
        KeyBinding::new("backspace", ClearCells, g),
        KeyBinding::new("ctrl-z", Undo, g),
        KeyBinding::new("ctrl-shift-z", Redo, g),
        KeyBinding::new("ctrl-y", Redo, g),
        KeyBinding::new("ctrl-n", NewFile, None),
        KeyBinding::new("ctrl-o", OpenFile, None),
        KeyBinding::new("ctrl-shift-s", SaveAs, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
}
