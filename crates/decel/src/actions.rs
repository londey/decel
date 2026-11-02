// SPDX-License-Identifier: MIT OR Apache-2.0

//! Actions, the command table and the default keymap (DESIGN §8.7).
//!
//! Every command lives in [`commands`]: it drives the key bindings, the
//! command palette and `[keymap]` overrides in `config.toml`.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{Action, App, KeyBinding, KeyBindingContextPredicate, actions};

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
        ToggleCodeSide,
        ToggleDock,
        ToggleBoundaries,
        FillDown,
        FillRight,
        AutoFillColumn,
        AutoFillRow,
        Cancel,
        CopyCells,
        CutCells,
        PasteCells,
        PasteValues,
        PasteMeta,
        NewMetaOnSelection,
        ToggleHistory,
        AddSheet,
        InsertCells,
        DeleteCells,
        InsertChart,
        ImportCsv,
        ExportCsv,
        CommandPalette,
        DetachMeta,
        OpenConfig,
    ]
);

/// The grid's key context.
pub const GRID: &str = "Grid";

/// One user-facing command.
pub struct CommandInfo {
    /// Name used in `config.toml` `[keymap]` (snake case).
    pub name: &'static str,
    /// Title shown in the command palette.
    pub title: &'static str,
    pub action: Box<dyn Action>,
    /// Default keystrokes.
    pub keys: &'static [&'static str],
    /// Bound only while the grid has focus.
    pub grid: bool,
    /// Listed in the command palette.
    pub palette: bool,
}

fn cmd(
    name: &'static str,
    title: &'static str,
    action: impl Action,
    keys: &'static [&'static str],
    grid: bool,
) -> CommandInfo {
    CommandInfo {
        name,
        title,
        action: Box::new(action),
        keys,
        grid,
        palette: true,
    }
}

/// Navigation commands bound to keys but not worth listing in the palette.
fn nav(name: &'static str, action: impl Action, keys: &'static [&'static str]) -> CommandInfo {
    CommandInfo {
        palette: false,
        ..cmd(name, name, action, keys, true)
    }
}

pub fn commands() -> Vec<CommandInfo> {
    vec![
        nav("move_up", MoveUp, &["up"]),
        nav("move_down", MoveDown, &["down"]),
        nav("move_left", MoveLeft, &["left"]),
        nav("move_right", MoveRight, &["right"]),
        nav("select_up", SelectUp, &["shift-up"]),
        nav("select_down", SelectDown, &["shift-down"]),
        nav("select_left", SelectLeft, &["shift-left"]),
        nav("select_right", SelectRight, &["shift-right"]),
        nav("jump_up", JumpUp, &["ctrl-up"]),
        nav("jump_down", JumpDown, &["ctrl-down"]),
        nav("jump_left", JumpLeft, &["ctrl-left"]),
        nav("jump_right", JumpRight, &["ctrl-right"]),
        nav("page_up", PageUp, &["pageup"]),
        nav("page_down", PageDown, &["pagedown"]),
        nav("row_start", RowStart, &["home"]),
        nav("row_end", RowEnd, &["end"]),
        nav("sheet_start", SheetStart, &["ctrl-home"]),
        nav("cancel", Cancel, &["escape"]),
        cmd(
            "command_palette",
            "Command palette",
            CommandPalette,
            &["ctrl-shift-p"],
            false,
        ),
        cmd("edit_cell", "Edit cell", EditCell, &["f2", "enter"], true),
        cmd(
            "clear_cells",
            "Clear cells / delete chart",
            ClearCells,
            &["delete", "backspace"],
            true,
        ),
        cmd("undo", "Undo", Undo, &["ctrl-z"], true),
        cmd("redo", "Redo", Redo, &["ctrl-shift-z", "ctrl-y"], true),
        cmd("new_file", "New workbook", NewFile, &["ctrl-n"], false),
        cmd("open_file", "Open workbook…", OpenFile, &["ctrl-o"], false),
        cmd("save_as", "Save as…", SaveAs, &["ctrl-shift-s"], false),
        cmd("import_csv", "Import CSV…", ImportCsv, &[], false),
        cmd("export_csv", "Export CSV…", ExportCsv, &[], false),
        cmd("quit", "Quit", Quit, &["ctrl-q"], false),
        cmd(
            "toggle_code_side",
            "Toggle code side",
            ToggleCodeSide,
            &["f3"],
            false,
        ),
        cmd(
            "toggle_metatable_dock",
            "Metatable dock",
            ToggleDock,
            &["f4"],
            false,
        ),
        cmd(
            "toggle_history_dock",
            "History dock",
            ToggleHistory,
            &["f5"],
            false,
        ),
        cmd(
            "toggle_boundaries",
            "Toggle metatable boundaries",
            ToggleBoundaries,
            &["ctrl-shift-b"],
            false,
        ),
        cmd(
            "new_meta_on_selection",
            "New metatable on selection",
            NewMetaOnSelection,
            &["ctrl-shift-m"],
            false,
        ),
        cmd(
            "detach_meta",
            "Detach selection onto a private metatable",
            DetachMeta,
            &[],
            true,
        ),
        cmd("fill_down", "Fill down", FillDown, &["ctrl-d"], true),
        cmd("fill_right", "Fill right", FillRight, &["ctrl-r"], true),
        cmd(
            "auto_fill_column",
            "Auto-fill column",
            AutoFillColumn,
            &["ctrl-shift-d"],
            true,
        ),
        cmd(
            "auto_fill_row",
            "Auto-fill row",
            AutoFillRow,
            &["ctrl-shift-r"],
            true,
        ),
        cmd("copy", "Copy", CopyCells, &["ctrl-c"], true),
        cmd("cut", "Cut", CutCells, &["ctrl-x"], true),
        cmd("paste", "Paste", PasteCells, &["ctrl-v"], true),
        cmd(
            "paste_values",
            "Paste values",
            PasteValues,
            &["ctrl-shift-v"],
            true,
        ),
        cmd(
            "paste_meta",
            "Paste metatables",
            PasteMeta,
            &["ctrl-alt-v"],
            true,
        ),
        cmd("add_sheet", "Add sheet", AddSheet, &["shift-f11"], false),
        cmd(
            "insert_cells",
            "Insert rows / columns",
            InsertCells,
            &["ctrl-shift-=", "ctrl-+", "ctrl-shift-+"],
            true,
        ),
        cmd(
            "delete_cells",
            "Delete rows / columns",
            DeleteCells,
            &["ctrl--"],
            true,
        ),
        cmd(
            "insert_chart",
            "Insert chart from selection",
            InsertChart,
            &["alt-f1", "f11"],
            true,
        ),
        cmd(
            "open_config",
            "Open configuration file",
            OpenConfig,
            &[],
            false,
        ),
    ]
}

fn binding(keys: &str, action: Box<dyn Action>, grid: bool, cx: &App) -> Option<KeyBinding> {
    let ctx = if grid {
        KeyBindingContextPredicate::parse(GRID).ok().map(Rc::new)
    } else {
        None
    };
    KeyBinding::load(keys, action, ctx, false, None, cx).ok()
}

/// Binds the default keymap plus `overrides` (action name → keystrokes).
/// Returns the names or keystrokes that could not be bound.
pub fn bind_keys(cx: &mut App, overrides: &BTreeMap<String, String>) -> Vec<String> {
    let cmds = commands();
    let mut bindings = Vec::new();
    for c in &cmds {
        for k in c.keys {
            if let Some(b) = binding(k, c.action.boxed_clone(), c.grid, cx) {
                bindings.push(b);
            }
        }
    }
    let mut bad = Vec::new();
    for (name, keys) in overrides {
        match cmds.iter().find(|c| c.name == name) {
            Some(c) => match binding(keys, c.action.boxed_clone(), c.grid, cx) {
                Some(b) => bindings.push(b),
                None => bad.push(format!("{name} = {keys:?}")),
            },
            None => bad.push(format!("unknown action {name:?}")),
        }
    }
    cx.bind_keys(bindings);
    bad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_unique_snake_case() {
        let cmds = commands();
        let mut names: Vec<&str> = cmds.iter().map(|c| c.name).collect();
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
        assert!(
            names
                .iter()
                .all(|n| n.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        );
    }
}
