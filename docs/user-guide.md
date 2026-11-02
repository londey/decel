# decel user guide

decel is a desktop spreadsheet. If you know Excel or LibreOffice Calc, most of it will feel
familiar: a grid of cells, sheets along the bottom, arrow keys to move, `Enter` to edit.

The difference is what a cell can hold. In Excel, a cell's formula, number format, validation
rule, conditional formatting and comment are spread over several dialogs. In decel all of them
live in one place: a small **Lua metatable** attached to the cell. You can see that metatable,
colour the grid by it, share it between thousands of cells, and look at its history.

This guide covers using the application. The Lua side (what you can write inside a metatable)
is described in the [Lua API reference](lua-api.md). Example workbooks to explore are described
in [`examples/README.md`](../examples/README.md).

## Contents

1. [Concepts](#1-concepts)
2. [The three modes](#2-the-three-modes-normal-peek-and-code-side)
3. [Entering and editing values](#3-entering-and-editing-values)
4. [The Metatable dock](#4-the-metatable-dock)
5. [Shared metatables: copy-on-write, shared edits and Detach](#5-shared-metatables-copy-on-write-shared-edits-and-detach)
6. [Filling](#6-filling)
7. [Clipboard](#7-clipboard)
8. [Sheets, rows and columns](#8-sheets-rows-and-columns)
9. [History and undo](#9-history-and-undo)
10. [Charts](#10-charts)
11. [CSV import and export](#11-csv-import-and-export)
12. [Files](#12-files)
13. [The command palette](#13-the-command-palette)
14. [Configuration](#14-configuration)
15. [Keyboard reference](#15-keyboard-reference)

---

## 1. Concepts

### 1.1 Cells: a raw value plus an optional metatable

Every cell has two optional parts:

| Part | What it is | Example |
|---|---|---|
| **Raw value** | What you typed, after parsing. A boolean, an integer, a decimal number or text. | `-35.3075` |
| **Metatable** | A set of Lua *members* that decide how the cell is computed, shown, edited, parsed, validated and described. | "show as degrees-minutes-seconds, edit as decimal, reject anything beyond ±90°" |

A cell with neither is empty. A cell can have a metatable but no raw value. A computed cell
(like a running total) is usually exactly that.

The value other cells see is the **final value**. If the metatable has a `value` member, the
final value is whatever that member computes. Otherwise it is the raw value.

decel has no formula bar language and no `A1` references. Calculations are written in Lua inside
a metatable, and they refer to other cells with calls such as `c:left()` (the cell to the left)
or `c:above():sum()` (everything above in this column). See the
[Lua API reference](lua-api.md).

Rows and columns are numbered from 1. The cell address in the edit bar is shown as
`row,col`, so `2,3` is row 2, column 3. Column headers show numbers by default; you can switch
them to letters in [the configuration](#14-configuration). Letters are for display only; Lua
code always uses numbers.

### 1.2 Structural identity

A metatable is identified by its **source code**, not by where it came from. decel computes a
hash (a fingerprint) of each metatable's members. Two cells whose metatables have exactly the
same source have the same hash, and decel treats them as the same metatable:

- they get the same colour in peek mode and on the code side,
- no boundary line is drawn between them,
- they are in the same **share set** (see [section 5](#5-shared-metatables-copy-on-write-shared-edits-and-detach)),
- the file stores the source only once.

Whitespace at the ends of lines and differences in line endings don't count, so reformatting
trailing spaces does not change a metatable's identity. Anything else does: renaming a helper or
changing a comment gives a new hash.

### 1.3 Colours and boundaries

Each distinct metatable in a workbook gets a colour from a palette of 16 hues, tuned separately
for light and dark themes. Colours are given out in order of first appearance (sheet order, then
row by row) and saved in the workbook, so they stay the same each time you open it. After 16
metatables the hues repeat with a hatch pattern so they remain distinguishable.

A thin **boundary** line is drawn between neighbouring cells whose metatables differ. It is grey
in normal mode and coloured in peek mode and on the code side. Toggle boundaries with
`Ctrl+Shift+B`.

### 1.4 The file is always saved

There is no Save command and no "unsaved changes" prompt. A `.decel` file is a SQLite database,
and every change is written to it the moment you make it. If decel or your computer crashes,
every edit you finished is still there when you reopen the file. **Save As** makes a copy under
a new name.

### 1.5 History is never lost

Every change is recorded as a transaction in the file's history, and nothing is ever deleted
from it. Undo doesn't erase anything: it records a new transaction that puts things back.
That means:

- undo and redo survive closing and reopening the file,
- every cell has a full history you can browse and restore from (`F5`),
- you can always get back to any earlier state of a cell.

---

## 2. The three modes: normal, peek and code side

| Mode | How to enter it | What you see | Clicking a cell |
|---|---|---|---|
| **Normal** | The default | Each cell's displayed value | Selects it. Typing or `F2` edits the value. |
| **Peek** | **Hold** `` ` `` (backtick). Release to leave. | Cells tinted by metatable colour, plus a floating **legend** listing each colour's kind, unit and cell count. Cells without a metatable are not tinted. | Same as normal |
| **Code side** | `F3` toggles it, with a short flip animation | Cells tinted like peek, showing their metatable's `kind` (or first member name) instead of their value. Boundaries are emphasised and the Metatable dock opens. | Opens that cell's metatable for editing. Hold `Shift` to see and edit the whole share set. |

Peek is a *quasimode*: it lasts only while you hold the key, like Shift. It's meant for a quick
"what kinds of cells are on this sheet?" glance. Switching to another window while holding it
releases it, so it never gets stuck. The peek key can be changed with `peek_key` in
[the configuration](#14-configuration).

The legend counts cells on the current sheet, and shows "+N elsewhere" when the same metatable
is also used on other sheets.

---

## 3. Entering and editing values

### 3.1 Typing

| Action | Keys |
|---|---|
| Start editing, replacing the content | Just start typing |
| Edit the current content | `F2`, `Enter` or double-click |
| Commit and move down / up | `Enter` / `Shift+Enter` |
| Commit and move right / left | `Tab` / `Shift+Tab` |
| Cancel the edit | `Esc` |
| Clear the selected cells (values and metatables) | `Delete` or `Backspace` |

When you press `F2` on a cell with a metatable, the text placed in the editor comes from the
metatable's `edit` member, if it has one. That lets a cell *show* one thing and *edit* another.
For example, a latitude can display as `35°18′27.0″S` but open in the editor as `-35.307500`.

### 3.2 How typed text becomes a value

When you commit an edit, decel turns the text into a raw value:

1. If the cell's metatable has a `parse` member, that decides. It can accept many input
   formats (for example a latitude typed either as `-35.3075` or as `35 18 27 S`), or reject
   the text with a message.
2. Otherwise the **default parser** is used:

   | Typed text | Becomes |
   |---|---|
   | (nothing) | The raw value is cleared |
   | `true`, `FALSE` (any case) | A boolean |
   | `42`, `-7`, `1,000`, `1_000` | An integer |
   | `3.14`, `-0.5`, `1e3`, `1,234.5` | A decimal number |
   | `'42` (leading apostrophe) | The text `42`, never a number |
   | anything else | Text |

3. If the metatable has a `validate` member, the parsed value is checked.

If parsing or validation fails, the edit is **rejected**: the editor stays open and the reason is
shown in red under the edit bar. Fix the text and press `Enter` again, or press `Esc` to give up
and leave the cell as it was.

Validation also runs after a computed cell recalculates. A failure there doesn't reject anything
(there is nothing to reject). Instead the cell gets a **warning marker**, a small triangle in its
corner, and the message appears in its tooltip.

### 3.3 Edit hints and tooltips

- While you edit a cell, the right side of the edit bar shows a hint from the metatable's
  `edit_hint` member, such as `Decimal degrees (−35.3075) or 35°18'27"S`.
- Hovering over a cell for about 400 ms shows a tooltip. It comes from the metatable's `tooltip`
  member if there is one. Otherwise it shows the value followed by the unit, such as `12.5 m`.
  Cells with errors show the error message and where it happened (member and line), and cells
  with a validation warning add the warning (`⚠ …`).

### 3.4 Errors

A cell whose Lua code fails shows an error code instead of a value:

| Code | Meaning |
|---|---|
| `#LUA` | The Lua code raised an error (a bug, `error(...)`, a nil where a number was needed…) |
| `#CYCLE` | The cell depends on itself, directly or through other cells |
| `#TIMEOUT` | The code ran too long (see `instruction_budget`) |
| `#MEMORY` | The code used too much memory (see `memory_limit_mb`) |
| `#REF` | A reference that can't be resolved, such as `c:sheet_named("Inputs")` when no sheet has that name |
| `#TYPE` | A `value` member returned something that can't be a cell value, such as a table |

Errors spread: a cell that reads an error cell gets the same error, like `#REF!` chains in
Excel. Hover over the cell to see the message. If a *display* member fails, the cell falls back to
its plain value with a red corner marker; display problems never break the grid.

---

## 4. The Metatable dock

The right-hand dock has tabs for **Metatable**, **History** and **Chart**. It is hidden until
you need it. `F4` toggles the Metatable tab, `F5` the History tab, and `F3` (the code side) opens
the Metatable tab automatically.

### 4.1 Layout

```
┌ Metatable ─────────────────────────────────┐
│ Editing this cell only · shared by 312     │  ← banner (and Detach button)
│ ┌─────────────┐ ┌────────────────────────┐ │
│ │✓ kind       │ │ function display(self, │ │  ← fixed signature
│ │✓ unit       │ │          c, v)         │ │
│ │✓ display •  │ │   return fmt.dms(v,    │ │  ← the body you edit
│ │✓ edit       │ │     "lat", 1)          │ │
│ │✓ parse      │ │ end                    │ │
│ │⚠ validate   │ ├────────────────────────┤ │
│ │  + add hook │ │ ✓ compiles · preview:  │ │  ← live compile status and preview
│ │  + helper   │ │ 35°18′27.0″S           │ │
│ └─────────────┘ └────────────────────────┘ │
│ log: (print output for this cell)          │
└────────────────────────────────────────────┘
```

### 4.2 Members

A metatable is a list of **members**, and you edit one member at a time.

| Kind | Examples | What you edit |
|---|---|---|
| **Fields** | `kind`, `unit`, `style` | A Lua expression, such as `"latitude"` or `{ align = "right" }` |
| **Hooks** | `value`, `display`, `edit`, `parse`, `validate`, `tooltip`, `edit_hint` | Only the function body. The signature (`function display(self, c, v)`) is fixed and shown read-only above it. |
| **Helpers** | any other name | A free-form Lua expression, usually `function(self, ...) ... end`, called from hooks as `self:name(...)` |

- **+ add hook** offers the standard hooks the metatable doesn't have yet.
- **+ helper** adds a free-form member.
- **✕** next to the selected member removes it.
- `Ctrl+Shift+M` (**New metatable on selection**) gives the selected cells a fresh metatable and
  opens the dock on its `kind` field.

Each member shows a status: **✓** it compiles, **⚠** it raised a runtime error on this cell,
**✗** it has a syntax error. Error messages name the member and the line within its body.

### 4.3 Drafts, commit and revert

Typing in the editor creates a **draft**. Members with drafts are marked with `•` in the list.
Drafts survive switching to another member, toggling the code side, and switching to another
application. Nothing in the workbook changes while you type.

| Action | Keys |
|---|---|
| Commit all drafts | `Ctrl+Enter`, or select another cell |
| Revert the selected member's draft | `Esc` |

A commit is a single transaction, so one `Ctrl+Z` undoes it.

### 4.4 Preview and print log

Below the editor, decel compiles your draft as you type and shows the result of running it
against the selected cell: the text the cell *would* display. This is a scratch evaluation and
never touches the workbook.

Anything your code passes to `print(...)` goes to the **log** at the bottom of the dock, for the
selected cell. It's the easiest way to debug a hook. (Up to 200 lines are kept per evaluation.)

---

## 5. Shared metatables: copy-on-write, shared edits and Detach

Because metatables are identified by their source (section 1.2), filling a metatable down 500
rows doesn't create 500 copies. All 500 cells point to one metatable, its **share set**. Editing
a shared metatable therefore needs a choice: change just this cell, or all of them?

### 5.1 Copy-on-write (the default)

Clicking a cell on the code side (or selecting it with the dock open) and editing its metatable
changes **only that cell**. When you commit, decel creates a new metatable (with a new hash and
colour) and points just that cell at it. The banner says so before you type:

> Editing this cell only · shared by 312 others

### 5.2 Shared edits (hold Shift)

To change every cell in a share set at once, use the code side:

1. Press `F3` to flip to the code side.
2. Hold `Shift` and hover over a cell. Its whole share set is outlined, with a count that includes
   cells on other sheets. This is exactly what a shared edit will change.
3. `Shift+click` the cell. The banner changes to, for example:

   > Editing 313 cells (3 on other sheets)

4. Edit and commit with `Ctrl+Enter`. Every cell in the share set moves to the new metatable in
   one transaction. They keep sharing one colour, and one `Ctrl+Z` undoes the whole change.

Because identity is structural, two cells you set up separately with identical code are in the
same share set. The Shift-hover outline always shows exactly what a shared edit will touch.

### 5.3 Detach

**Detach** (a button in the Metatable dock banner when the metatable is shared) gives the selected
cells their own private copy of the metatable without changing anything else. The copy gets a new
colour, and you can later edit those cells as a group of their own.

Because identity comes from source code, a private copy has to differ in source. Detach adds a
helper member called `_copy` with a number in it. You can leave it alone.

---

## 6. Filling

Filling copies **metatables** into other cells. The destination cells keep their own raw values
unless you ask for values too.

| Operation | How | What happens |
|---|---|---|
| **Fill drag** | Drag the small square (the *fill handle*) at the corner of the selection | Repeats the selection's metatables over the dragged area, showing a preview outline while you drag. Hold `Ctrl` while dragging to copy raw values as well. |
| **Fill down** | `Ctrl+D` | Copies the first row of the selection over the rest of the selection |
| **Fill right** | `Ctrl+R` | Copies the first column of the selection over the rest |
| **Auto-fill column** | `Ctrl+Shift+D` | See below |
| **Auto-fill row** | `Ctrl+Shift+R` | The same, along the row |

Filled cells all share one metatable (and one colour), so a fill of 10 000 cells costs one stored
metatable. A fill is one transaction and undoes in one step.

**Auto-fill** saves you from selecting a run by hand. From the selected cell, decel finds the
**adjacent run** above and below it (for auto-fill column): the unbroken block of non-empty
cells next to it that all share the same metatable as its immediate neighbour (or that all have
none). The run is outlined in amber and the status bar asks for confirmation:

- `Enter` applies the selected cell's metatable to the whole run.
- `Esc` cancels.

A typical use: type numbers in a column, put a running-total metatable in the cell next to the
first number, then auto-fill the column to cover every row.

---

## 7. Clipboard

| Action | Keys | Effect |
|---|---|---|
| Copy | `Ctrl+C` | Copies values and metatables. Other applications receive tab-separated text (each cell's edit text). |
| Cut | `Ctrl+X` | Copies, then clears the source cells |
| Paste | `Ctrl+V` | Pastes raw values **and** metatables. Pasted cells share metatables with the source (same colour). |
| Paste values | `Ctrl+Shift+V` | Raw values only; the destination cells keep their metatables |
| Paste metatables | `Ctrl+Alt+V` | Metatables only; the destination cells keep their values |

Text copied from another application (for example a column from a web page or another
spreadsheet) is pasted cell by cell **through each destination cell's `parse` and `validate`**,
exactly as if you had typed it. Values that are rejected are left unchanged.

---

## 8. Sheets, rows and columns

### 8.1 Sheets

| Action | How |
|---|---|
| Add a sheet | **+** after the last tab, or `Shift+F11` |
| Rename | Double-click the tab, type, `Enter` (or `Esc` to cancel) |
| Reorder | Drag a tab onto another |
| Delete | **✕** on the active tab, then confirm |

All of these are recorded in history and can be undone.

Lua code can read other sheets by name with `c:sheet_named("Inputs")`. If you rename or delete
that sheet, those cells show `#REF`. They recover by themselves if a sheet with that name
appears again.

### 8.2 Inserting and deleting rows and columns

| Action | Keys |
|---|---|
| Insert rows (as many as are selected, above the selection) | `Ctrl+Shift+=` (also `Ctrl++`) |
| Delete the selected rows | `Ctrl+-` |

Select whole rows by clicking (or Shift-clicking) the row headers. If whole **columns** are
selected (via the column headers), the same keys insert or delete columns instead. With an
ordinary cell selection, the keys act on the selection's rows.

When rows or columns move, decel moves everything with them: cells, column widths and row
heights, chart anchors and chart ranges, and per-cell history.

What it does **not** rewrite is Lua code. This matters for references:

- **Relative references keep working.** `c:left()`, `c:up(2)`, `c:above()`, `c:offset(-1, 0)` and
  similar are relative to the evaluating cell, so they still point at the same neighbours after
  the cells move.
- **Absolute references are not adjusted.** `c:at(5, 2)` and `sheet:at(5, 2)` always mean row 5,
  column 2, whatever is there now. Insert a row above row 5 and the code reads the new, blank row
  5. Prefer relative references, or look cells up by a label, when the layout may change.

For example, inserting three blank rows in the middle of a running total keeps the totals above
the gap. Below the gap the totals restart, because a running total looks at `c:up()`, which is
now a blank row.

---

## 9. History and undo

### 9.1 Undo and redo

| Action | Keys |
|---|---|
| Undo | `Ctrl+Z` |
| Redo | `Ctrl+Shift+Z` or `Ctrl+Y` |

Each user action (an edit, a fill, a metatable commit, an import, a row insert…) is one
transaction and undoes in one step. The status bar shows what the next undo and redo will do.

Undo history is stored in the file, so you can close decel, reopen the workbook next week, and
still undo. Making a new change after undoing clears the redo list, as usual, but the undone
transactions remain in the file's history.

### 9.2 The History dock (`F5`)

The History tab has two parts.

**Cell history** lists every version of the selected cell, newest first: time, a label (such as
"Fill down 500 cells"), and `before → after`. Versions where the metatable changed carry a
**metatable** badge; expand one to see a member-by-member diff (changed members side by side,
added and removed members flagged, unchanged ones collapsed). **Restore this version** puts the
cell back to that version as a new, undoable transaction. Cell history follows the cell through
row and column inserts, so earlier versions are still listed after the cell has moved.

**Undo history** lists the transactions you can undo (the current position is marked) and the ones
you can redo (shown in italics). Click an entry to undo or redo everything up to that point.
The first entry, "(opened workbook / start)", undoes everything.

---

## 10. Charts

1. Select the data, with a header row if you have one. The first column is used for X and each
   further column becomes a series.
2. Press `Alt+F1` or `F11`.

A floating line chart appears, anchored two columns to the right of the selection so it doesn't
cover the data. Charts update live as values recalculate, even when they are off-screen.

- **Select** a chart by clicking it. This opens the **Chart** tab in the dock.
- **Move** it by dragging; it snaps to the cell under its top-left corner.
- **Resize** it with the grip at the bottom-right corner.
- **Delete** the selected chart with `Delete`.

The Chart tab sets the type (line, bar or scatter), the chart and axis titles (**Apply titles**),
and the legend. **Use selection** sets the X range or a series range from the current grid
selection, **+ Add series from selection** adds a series, and clicking a series swatch cycles its
colour. Text in a series range is skipped, and the chart shows "N non-numeric skipped" in its
corner.

Charts stay anchored to their cell when you insert or delete rows and columns, and their ranges
grow or shrink with the data. Every chart change can be undone.

---

## 11. CSV import and export

Both live in the file menu (click the title at the top of the window).

### 11.1 Import CSV…

You can also drop a `.csv` file on the window, or start decel with `decel data.csv`.

| Option | Choices |
|---|---|
| Destination | A **new sheet** named after the file, or **at the selection** over existing cells |
| Header row | Whether the first line is a header (it is written as plain text) |
| Delimiter | Auto, `,`, `;`, Tab or `|` |
| Encoding | Auto, UTF-8 or Windows-1252 |

- **Auto delimiter** looks at the first 50 records and picks the delimiter that gives the most
  consistent field counts. European files such as `1,5;2,25` are read as semicolon-separated.
- **Auto encoding** reads UTF-8 (a byte-order mark is removed) and falls back to Windows-1252 if
  the file isn't valid UTF-8, so `é` and `€` from older Excel exports come through correctly.
- Into a **new sheet**, values go through the default parser (section 3.2).
- **At the selection**, each value goes through the target cell's `parse` and `validate`, as if
  typed. Rejected values are left unchanged and counted in the status bar.

An import is one transaction, including the new sheet, so a single `Ctrl+Z` removes it.

### 11.2 Export CSV…

Exports the whole sheet or the selection, with the delimiter and quoting you choose, as either:

- **raw values** in edit form, which re-import to identical values, or
- **displayed text**, exactly as shown in the grid (`A$1,234.50`, `35°18′27″S`…).

---

## 12. Files

| Action | Keys / where |
|---|---|
| New workbook | `Ctrl+N` |
| Open | `Ctrl+O`, or `decel path/to/file.decel` |
| Save As | `Ctrl+Shift+S` |
| Recent files | The file menu (click the window title) |
| Quit | `Ctrl+Q` |

**Untitled workbooks.** A new workbook is real from the first keystroke: it is kept in decel's
data directory (`untitled/`) until you use **Save As** to give it a name and location. Untitled
workbooks that you never edited are cleaned up the next time decel starts. The window title shows
"Untitled" until you save.

**Save As** writes a compact copy of the workbook, including its full history, to a new file
and continues in that file. It won't overwrite an existing file; pick a new name or delete
the old file first.

**One writer at a time.** While a workbook is open, decel keeps a lock file next to it
(`name.decel.lock`, recording the process and computer). If you open a file that is already open
in another decel, a banner says where it is open and offers **Open read-only**. In read-only mode
a banner reminds you that changes can't be saved, and edits are refused.

---

## 13. The command palette

decel has no ribbon and no toolbar. Every command is in the **command palette**:

1. Press `Ctrl+Shift+P`.
2. Type part of a command's name. The search is fuzzy, so `fd` finds **Fill down** and `pv`
   finds **Paste values**.
3. Use the arrow keys to choose and `Enter` to run it, or `Esc` to close the palette.

The palette lists every command, with its current keyboard shortcut, so it's also a quick way to
look up a key.

---

## 14. Configuration

decel reads an optional `config.toml`:

| Platform | Location |
|---|---|
| Linux | `$XDG_CONFIG_HOME/decel/config.toml` (usually `~/.config/decel/config.toml`) |
| Windows | `%APPDATA%\decel\config.toml` |

The file is **hot-reloaded**: save it and decel applies the changes straight away. If the file
has an error (bad TOML, an unknown value, an unknown action name), decel shows a notification
describing the problem and otherwise ignores the invalid setting, keeping its previous or default
value.

| Key | Values | Default | Meaning |
|---|---|---|---|
| `theme` | `"system"`, `"light"`, `"dark"` | `"system"` | Colour theme. `"system"` follows the OS, including when it changes. |
| `font_family` | a font name | the system UI font | Font used in the grid |
| `font_size` | a number (points) | platform default | Grid font size |
| `peek_key` | a key name | `` "`" `` | The key you hold for peek mode |
| `column_headers` | `"numbers"`, `"letters"` | `"numbers"` | Show column headers as `1 2 3` or `A B C`. Display only: Lua always uses numbers. |
| `instruction_budget` | an integer | `10_000_000` | Lua VM instructions one evaluation may run before the cell shows `#TIMEOUT` |
| `memory_limit_mb` | an integer | `512` | Memory the Lua engine may use, in MiB, before an evaluation fails with `#MEMORY` |
| `[keymap]` | a table | see [section 15](#15-keyboard-reference) | Keyboard shortcut overrides |

### 14.1 The `[keymap]` table

Each entry maps a **command name** (the names in the "Action" column of
[the keyboard reference](#15-keyboard-reference)) to a keystroke. A keystroke is written as
modifiers and a key joined by `-`, in lower case: `ctrl-`, `shift-`, `alt-`, then the key
(`a`, `f2`, `enter`, `pageup`, `=`, `-`…). An entry adds a binding; the default binding
keeps working too. Unknown names or keystrokes are reported in a banner.

```toml
# ~/.config/decel/config.toml
theme = "dark"
font_family = "JetBrains Mono"
font_size = 13
peek_key = "§"
column_headers = "letters"
instruction_budget = 20_000_000
memory_limit_mb = 1024

[keymap]
command_palette = "ctrl-shift-p"
fill_down = "ctrl-d"
toggle_code_side = "f9"
insert_chart = "ctrl-shift-c"
```

---

## 15. Keyboard reference

These are the default bindings. The **Action** column is the command name to use in
[`[keymap]`](#141-the-keymap-table). "Grid" means the binding applies while the grid has focus;
"Global" means it works anywhere in the window.

### 15.1 Navigation and selection (grid)

| Action | Keys | Does |
|---|---|---|
| `move_up` / `move_down` / `move_left` / `move_right` | `↑` `↓` `←` `→` | Move the cursor |
| `select_up` / `select_down` / `select_left` / `select_right` | `Shift+↑` `Shift+↓` `Shift+←` `Shift+→` | Extend the selection |
| `jump_up` / `jump_down` / `jump_left` / `jump_right` | `Ctrl+↑` `Ctrl+↓` `Ctrl+←` `Ctrl+→` | Jump to the edge of the current data run |
| `page_up` / `page_down` | `PgUp` / `PgDn` | Move a screen up or down |
| `row_start` / `row_end` | `Home` / `End` | First / last column of the row |
| `sheet_start` | `Ctrl+Home` | Go to cell 1,1 |

Mouse: click to select, drag to select a rectangle (the grid auto-scrolls at the edges), click a
row or column header to select the whole row or column, `Shift+click` to extend. The mouse wheel
scrolls vertically; `Shift`+wheel or a touchpad scrolls horizontally. Drag a header edge to resize
a column or row; double-click the edge to fit the contents.

### 15.2 Editing (grid)

| Action | Keys | Does |
|---|---|---|
| `edit_cell` | `F2`, `Enter` (also double-click, or just type) | Edit the cell |
| `clear_cells` | `Delete`, `Backspace` | Clear the selected cells |
| `undo` | `Ctrl+Z` | Undo |
| `redo` | `Ctrl+Shift+Z`, `Ctrl+Y` | Redo |
| `cancel` | `Esc` | Cancel the current operation (such as an auto-fill preview) |

While the cell editor is open: `Enter` / `Shift+Enter` commit and move down / up, `Tab` /
`Shift+Tab` commit and move right / left, `Esc` cancels.

### 15.3 Fill and clipboard (grid)

| Action | Keys | Does |
|---|---|---|
| `fill_down` | `Ctrl+D` | Fill down |
| `fill_right` | `Ctrl+R` | Fill right |
| `auto_fill_column` | `Ctrl+Shift+D` | Auto-fill the adjacent run in the column (preview, then `Enter`) |
| `auto_fill_row` | `Ctrl+Shift+R` | Auto-fill the adjacent run in the row |
| `copy` | `Ctrl+C` | Copy |
| `cut` | `Ctrl+X` | Cut |
| `paste` | `Ctrl+V` | Paste values and metatables |
| `paste_values` | `Ctrl+Shift+V` | Paste values only |
| `paste_meta` | `Ctrl+Alt+V` | Paste metatables only |

Fill handle: drag to fill metatables; hold `Ctrl` while dragging to fill values too.

### 15.4 Structure and charts (grid)

| Action | Keys | Does |
|---|---|---|
| `insert_cells` | `Ctrl+Shift+=`, `Ctrl++`, `Ctrl+Shift++` | Insert rows (or columns, if whole columns are selected) |
| `delete_cells` | `Ctrl+-` | Delete the selected rows (or columns) |
| `insert_chart` | `Alt+F1`, `F11` | Insert a chart from the selection |

### 15.5 Metatables, modes and panels (global)

| Action | Keys | Does |
|---|---|---|
| `command_palette` | `Ctrl+Shift+P` | Open the command palette |
| `toggle_code_side` | `F3` | Flip between normal mode and the code side |
| `toggle_metatable_dock` | `F4` | Show or hide the Metatable dock |
| `toggle_history_dock` | `F5` | Show or hide the History dock |
| `toggle_boundaries` | `Ctrl+Shift+B` | Show or hide metatable boundaries |
| `new_meta_on_selection` | `Ctrl+Shift+M` | Give the selection a new metatable and open it in the dock |
| `add_sheet` | `Shift+F11` | Add a sheet |

Not actions, but worth knowing:

| Key | Where | Does |
|---|---|---|
| `` ` `` (hold) | Grid | Peek mode (configurable with `peek_key`) |
| `Shift` (hold) | Code side | Show the share set under the mouse; `Shift+click` opens a shared edit |
| `Ctrl+Enter` | Metatable dock | Commit drafts |
| `Esc` | Metatable dock | Revert the selected member's draft |
| `Delete` | A selected chart | Delete the chart |

### 15.6 Files (global)

| Action | Keys | Does |
|---|---|---|
| `new_file` | `Ctrl+N` | New workbook |
| `open_file` | `Ctrl+O` | Open a workbook |
| `save_as` | `Ctrl+Shift+S` | Save As |
| `quit` | `Ctrl+Q` | Quit |
| `import_csv` | (palette / file menu) | Import CSV |
| `export_csv` | (palette / file menu) | Export CSV |
