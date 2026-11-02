# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Phase 7 — Sheets & structure

#### Added

- Sheet tabs: add, inline rename, drag to reorder, delete with confirmation (all undoable).
- Insert/delete rows and columns at the selection (`Ctrl+Shift+=` / `Ctrl+-`); cells, layout,
  chart anchors and history coordinates shift with them. Absolute `c:at()` is not rewritten.
- Engine tests for inserting inside a 500-row running total and deleting rows inside range
  dependencies; `testkit::Wb::structure`.

#### Fixed

- A full display snapshot now replaces stale cached cells (seen after structural edits).

### Phase 6 — History UI

#### Added

- History dock (`F5`): the selected cell's versions with time, label, before → after and a
  metatable badge; member-level diffs (`decel_model::diff_members`); restore.
- Undo history list with the current position; clicking undoes/redoes to that point.
- Status bar shows undo/redo availability and labels.
- Per-cell history bench: ~50 µs with 1 M `cell_changes` rows.

### Phase 5 — Metatable UX

#### Added

- Per-metatable colours from a 16-hue OKLCH palette (light/dark), assigned by first appearance
  and stored in the workbook; hatching beyond 16.
- Peek quasimode (hold `` ` ``) with a legend; automatic metatable boundaries (`Ctrl+Shift+B`).
- The code side (`F3`) with a flip animation, and the Metatable dock (`F4`): member list with
  compile/runtime status, add hook/helper, signature header, Lua editor with per-member drafts,
  live preview and `print` log, commit (`Ctrl+Enter`) and revert (`Esc`).
- Copy-on-write edits by default; Shift-hover share-set outline with counts; Shift+click shared
  edits; Detach.
- Fill handle drag, fill down/right, auto-fill column/row with a confirmable preview.
- Internal clipboard (values + metatables) and system clipboard as TSV; paste, paste values,
  paste meta; external text goes through `parse`/`validate`.
- "New metatable on selection" (`Ctrl+Shift+M`).

### Phase 4 — Grid UI

#### Added

- The decel window: client-side title bar with a file menu and recent files, edit bar (address,
  edit text, hint, rejection message), virtualised custom-painted grid with frozen headers, sheet
  tabs and a status bar (undo label, recalc progress, optional FPS/latency overlay with
  `DECEL_DEBUG_FPS=1`).
- Grid rendering: style colours and alignment, text spill into empty neighbours, ellipsis,
  error/warning corner markers, selection and cursor.
- Keyboard navigation (arrows, Ctrl+arrows to data edges, PgUp/PgDn, Home/End), mouse and drag
  selection with auto-scroll, whole row/column selection.
- In-cell editing with type-to-edit, F2/double-click, Enter/Tab/Esc, rejection display and
  optimistic updates; hover tooltips; column/row resize and double-click auto-fit.
- New/Open/Save As, recent files, untitled workbooks kept on disk, lock banner with read-only
  open, theme following the OS.
- `decel-cli demo` and `decel-cli gen-big`; `scripts/ui-drive.sh` for headless UI screenshots.

### Phase 3 — Engine actor

#### Added

- `decel_engine::protocol`: `Command`/`Reply`/`Event` types. `EngineHandle` runs the store and
  engine on their own thread, with 5 ms pre-emptible recalc slices, viewport subscriptions with
  coalesced display updates, and an idle WAL checkpoint.
- Fill drag/down/right and auto-fill run detection with preview; copy-on-write, shared and
  detach metatable edits; copy and paste (all/values/meta).
- `decel-cli eval file.decel [--sheet S] [--dump csv|values]` (dev only, not published).

#### Fixed

- Range dependencies covering a whole sheet overflowed the R-tree's area arithmetic.

### Phase 2 — Lua engine

#### Added

- Sandbox: frozen read-only globals, instruction budget plus wall-clock safety net, memory cap,
  interrupt-safe `pcall`/`xpcall`, `print` captured per cell.
- `fmt`, `parse`, `color` and `range` modules with golden tests.
- Metatable compilation per member (errors report member and line), frozen `self`.
- The `c` API: cell context, cell/range/sheet handles with runtime dependency recording;
  range dependencies in a per-sheet R-tree; sheet-name dependencies.
- Incremental recalculation with an explicit work stack (no Lua recursion), cycle detection,
  error propagation, validation warnings, and the display/tooltip/edit/parse/validate/preview
  pipelines.
- `decel_engine::testkit`: a headless DSL for building workbooks in tests.
- Differential fuzz test (incremental vs full recalc) and recalc benches.

### Phase 1 — Model & store

#### Added

- `decel-model`: `CellAddr`, `Rect`, `Value`/`CellError`, `Cell`, metatable sources with
  canonical form and BLAKE3 `MetaHash`, sparse `CellStorage`, `Op`/`Change` with inverses, an
  in-memory `Workbook` that applies changes, and the default value parser (§3.3).
- `decel-store`: SQLite schema v1 with a migration framework, `application_id` check, WAL mode,
  single-writer lock file and read-only fallback.
- Transactions that record before/after images; persistent undo/redo/restore rebuilt from the
  txn log on open; per-cell history that follows cells through row/column inserts; share-set
  and metatable queries; Save As via `VACUUM INTO`.
- Property tests (undo-all/redo-all, reopen, history replay) and a criterion bench for 10 k-cell
  commits and undo.

### Phase 0 — Foundations & risk spikes

#### Added

- Cargo workspace with the `decel-model`, `decel-store`, `decel-engine`, `decel-io` and `decel` crates.
- Dual MIT / Apache-2.0 licensing, SPDX headers, and `cargo-deny` licence and source policy.
- GitHub Actions CI on Linux and Windows: fmt, clippy, tests, cargo-deny, an install check and `cargo publish --dry-run`.
- Devcontainer with the GPUI Linux toolchain.
- `decel-engine::vm`: a Lua 5.4 VM that can move between threads, with a wall-clock budget
  enforced by an instruction hook and a hard memory cap. `pcall`/`xpcall` can't swallow
  limit errors.
- Spike window: a custom-painted, virtualised 1 000 × 50 grid and a Lua code editor built with gpui-component.
