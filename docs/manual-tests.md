# Manual tests

Checks that can't be automated (windowing, rendering, input). Run each section
on **Linux (Wayland)**, **Linux (X11)** and **Windows 11 (MSVC)** unless it says
otherwise, and record the date and result.

## Phase 0 — Spike window (Spike A / Spike C)

1. Install from the checkout:
   ```sh
   cargo install --path crates/decel --locked
   ```
   - [ ] It builds with only the README prerequisites installed.
2. Run `decel`.
   - [ ] A window opens at about 1280 × 800.
   - [ ] The left pane shows a grid with column headers `A`, `B`, … and row numbers `1`, `2`, ….
   - [ ] Cells show `row × column` (for example, `C4` shows `12`).
   - [ ] Scrolling with the mouse wheel or touchpad moves the grid. Headers stay fixed.
         Scrolling to the end reaches row 1000 and column `AX` (50 columns).
   - [ ] The right pane shows a Lua code editor with line numbers.
   - [ ] Lua is syntax highlighted: keywords (`return`, `function`, `local`, `if`), strings and
         `--` comments have distinct colours.
   - [ ] Typing in the editor works, and highlighting updates as you type.
3. Close the window.
   - [ ] The process exits cleanly.

| Date | OS / session | Result | Notes |
|---|---|---|---|
| | Linux Wayland | | |
| | Linux X11 | | |
| | Windows 11 | | |

## Phase 4 — Grid UI

Setup: `cargo run -p decel-cli -- demo /tmp/demo.decel`, then `cargo run --release -p decel -- /tmp/demo.decel`.
Run on **Wayland**, **X11** and **Windows**.

### Display and editing (DESIGN §4.7, §4.8)

- [ ] Latitudes in column 2 show as D°M′S″ (`35°18′27.0″S`), right-aligned. Longitudes that don't fit end with `…`.
- [ ] Running totals in column 6 read 10, 30, 60 … 2100.
- [ ] Row 23's long text spills across the empty cells to its right.
- [ ] Selecting 2,2 shows `-35.307500` in the edit bar with the hint `Decimal degrees (−35.3075) or 35°18'27"S`.
- [ ] Hovering 2,2 for about half a second shows `-35.307500° — 35°18′27.00″S`.
- [ ] `F2` (or double-click) on 2,2 edits `-35.307500`. Typing `95` and `Enter` keeps the editor open and shows `Latitude must be within ±90°` in red. `Esc` cancels, and the cell still shows the DMS value.
- [ ] Typing `35 18 27 S` into 2,2 and pressing `Enter` is accepted.
- [ ] Typing into an empty cell starts an edit. `Enter` commits and moves down, `Tab` moves right, `Shift+Enter`/`Shift+Tab` move back.
- [ ] Changing an Amount updates every running total below it.
- [ ] `Delete` clears the selection. `Ctrl+Z` / `Ctrl+Shift+Z` (or `Ctrl+Y`) undo and redo it.

### Navigation and selection

- [ ] Arrows, `Shift+arrows`, `PgUp`/`PgDn`, `Home`, `End`, `Ctrl+Home` behave as in other spreadsheets.
- [ ] `Ctrl+arrows` jump to the edges of data runs.
- [ ] Clicking a row/column header selects the whole row/column. Shift-click extends.
- [ ] Dragging selects a rectangle and auto-scrolls past the edge of the grid.
- [ ] The mouse wheel scrolls vertically, and `Shift`+wheel or a touchpad scrolls horizontally. Headers stay fixed.

### Layout and files

- [ ] Dragging a column header edge resizes the column. Double-clicking the edge auto-fits it. Row header edges resize rows. Sizes persist after reopening.
- [ ] Clicking the title opens New / Open / Save as and the recent files. `Ctrl+N`, `Ctrl+O` and `Ctrl+Shift+S` work, and the window title shows the file name.
- [ ] `decel path/to/file.decel` opens that file. With no argument, an untitled workbook opens, and untitled workbooks that were never edited are cleaned up on the next start.
- [ ] Opening a file already open in another decel shows a banner, and **Open read-only** opens it read-only (edits are refused).
- [ ] Kill the process (`kill -9`, or End Task on Windows) straight after an edit. Reopening shows every committed edit, and undo still works.
- [ ] Light and dark themes follow the OS setting, including switching while running.

### Performance (release build, real GPU)

1. `cargo run --release -p decel-cli -- gen-big /tmp/big.decel 1000000 20` (a large file, several GB).
2. `DECEL_DEBUG_FPS=1 cargo run --release -p decel -- /tmp/big.decel`
- [ ] The status bar shows ≥ 120 fps while scrolling continuously.
- [ ] `key→paint` stays under 8 ms while moving with the arrow keys.

| Date | OS / session | Result | Notes |
|---|---|---|---|
| | Linux Wayland | | |
| | Linux X11 | | |
| | Windows 11 | | |

> UI changes can also be checked headlessly in the devcontainer with
> `scripts/ui-drive.sh` (Xvfb + lavapipe + xdotool), which takes screenshots.

## Phase 5 — Metatable UX

Setup: `decel-cli demo /tmp/demo.decel` and open it.

### Walkthrough: build §4.7 and §4.8 through the UI only

1. In a new workbook, select 2,2 and press `Ctrl+Shift+M` (New metatable on selection). The dock opens on `kind`.
2. Set `kind` to `"latitude"`, then use **+ add hook** to add `unit`, `display`, `edit`, `parse`, `validate`, `tooltip` and `edit_hint` with the bodies from DESIGN §4.7. Watch the member statuses (✓/⚠/✗) and the live `preview:` line as you type. `Ctrl+Enter` commits.
3. Type `-35.3075` into 2,2: it shows `35°18′27.0″S`. Typing `95` is rejected with the message.
4. Drag the fill handle (the small square at the selection's corner) down to 2,10. All of them share one colour in peek.
5. Put numbers in column 5 (rows 2–20). Select 2,6, create a metatable with `kind = "running total"` and the §4.8 `value` and `style`, commit, then `Ctrl+D` over 2,6:20,6. The totals are correct.
- [ ] Every step works without the test harness.

### Peek, boundaries and code side

- [ ] Holding `` ` `` tints cells by metatable within a frame, with a legend (swatch, kind, unit, count). Releasing it clears immediately. Alt-tabbing away while holding doesn't leave it stuck.
- [ ] Boundaries are drawn between cells whose metatables differ (grey normally, coloured in peek). `Ctrl+Shift+B` toggles them.
- [ ] `F3` flips to the code side with a quick horizontal flip: cells show their `kind`, tinted, and the dock opens. `F3` flips back.
- [ ] More than 16 distinct metatables: colours repeat with a hatch.

### Copy-on-write and shared edits

- [ ] On the code side, clicking one of 20 running-total cells shows "Editing this cell only · shared by 19 others". Changing `kind` and pressing `Ctrl+Enter` changes only that cell's colour.
- [ ] Holding `Shift` and hovering a running-total cell outlines all 20 cells with a count (other sheets included). `Shift+click` shows "Editing 20 cells (…)". A commit changes all of them, their colour stays shared, and one `Ctrl+Z` undoes it.
- [ ] **Detach** gives the cell a private copy (a new colour) without changing the others.

### Editor drafts

- [ ] Type in `display`, switch to `value` and back: the typed text is still there (marked `•`). The same holds after toggling `F3` and after switching to another app and back.
- [ ] `Esc` in the editor reverts the selected member. Selecting another cell commits pending drafts.

### Fill, auto-fill and clipboard

- [ ] Fill-handle drag shows a preview outline and fills metatables (values with `Ctrl` held). `Ctrl+D` / `Ctrl+R` fill down/right.
- [ ] `Ctrl+Shift+D` on a cell above a run outlines the run in amber with a prompt in the status bar. `Enter` applies it, `Esc` cancels, and undo restores it in one step.
- [ ] `Ctrl+C` / `Ctrl+V` within decel pastes values and metatables (pasted cells share the source's colour). `Ctrl+Shift+V` pastes values only, and `Ctrl+Alt+V` metatables only.
- [ ] Copying to another app gives tab-separated edit text. Pasting text from another app goes through each target cell's `parse`/`validate`.
