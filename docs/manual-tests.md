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
