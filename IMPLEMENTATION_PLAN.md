# decel — Phased Implementation Plan (for Claude Code)

This plan takes decel from an empty repository to a published **v1.0** that installs with `cargo install decel` on Linux and Windows. It is written for Claude Code to carry out one phase at a time. `DESIGN.md` is the specification. Section references like **§5.3** point into it.

---

## How to use this plan

1. Put `DESIGN.md`, `IMPLEMENTATION_PLAN.md` and `CLAUDE.md` (template in Phase 0) in the repo root.
2. Start each phase in Claude Code with:
   > Read CLAUDE.md, DESIGN.md and Phase N of IMPLEMENTATION_PLAN.md. Use plan mode to propose the task breakdown, then implement Phase N. Do not start Phase N+1.
3. A phase is **done** when every acceptance criterion passes and CI is green. Tag it `phase-N`.
4. If implementation has to deviate from `DESIGN.md`, update the design document's decision table (§2) in the same PR and give the reason. The design document must always describe what was actually built.

### Rules for every phase

- The engine and store are **headless and test-first**. Logic is never tested only through the UI.
- No `unwrap()`/`expect()` outside tests and `main` setup. Errors use `thiserror` in library crates and `anyhow` only in the binary.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` and `cargo deny check` must all pass.
- No git dependencies (they break `cargo install` from crates.io). `cargo publish --workspace --dry-run` must pass.
- Every source file starts with `// SPDX-License-Identifier: MIT OR Apache-2.0`.
- Commits are small and conventional (`feat(engine): …`). Each phase ends with an updated `CHANGELOG.md`.
- Never block the UI thread on the engine. All engine access goes through commands and events (§9.2).

### Phase overview

| Phase | Name | Main output | Depends on |
|---|---|---|---|
| 0 | Foundations & risk spikes | Workspace, CI, devcontainer, GPUI and mlua proven on Linux and Windows | — |
| 1 | Model & store | Types, SQLite schema, transactions, undo/redo, history queries | 0 |
| 2 | Lua engine | Sandbox, metatables, `c` API, dependency tracking, recalculation | 1 |
| 3 | Engine actor | Threaded engine, command/event protocol, viewport subscriptions, benchmarks | 2 |
| 4 | Grid UI | Window, virtualised grid, selection, value editing, sheet tabs, files | 3 |
| 5 | Metatable UX | Peek, code side, boundaries, REALbasic editor, COW/shared edit, fill, clipboard | 4 |
| 6 | History UI | Undo/redo wiring, per-cell history panel, member diffs, restore | 5 |
| 7 | Sheets & structure | Sheet management, insert/delete rows and columns, cross-sheet references | 6 |
| 8 | CSV | Import/export | 7 |
| 9 | Charts | Floating line, bar and scatter charts with a config panel | 7 |
| 10 | Polish & release | Palette, config, themes, performance pass, docs, publish 1.0.0 | 8, 9 |

Phases 8 and 9 are independent and can run in parallel worktrees.

---

## Phase 0 — Foundations & risk spikes

**Goal:** prove the risky parts of the toolchain before writing any product code.

### Tasks

1. **Workspace skeleton** (§9.1): `crates/decel-model`, `decel-store`, `decel-engine`, `decel-io`, `decel`. Use workspace-level `[workspace.package]` (version `0.1.0`, edition 2024, `license = "MIT OR Apache-2.0"`, `repository`, `rust-version`) and `[workspace.dependencies]` with exact versions for GPUI-related crates.
2. **Licence files:** `LICENSE-MIT`, `LICENSE-APACHE`, README licence section, SPDX headers.
3. **`deny.toml`:** licence allowlist (§10.3), ban duplicate major versions of `windows`/`objc` where practical (warn only), deny git sources.
4. **CI** (Forgejo Actions, GitHub-Actions-compatible YAML in `.forgejo/workflows/ci.yml`). Jobs: fmt, clippy, test, deny, `publish --dry-run`. The matrix is `ubuntu-latest` plus a Windows runner (document how to register a Windows runner on the Legion laptop; mark the Windows job `continue-on-error` until a runner exists).
5. **Devcontainer** (`.devcontainer/`): Rust stable, the GPUI Linux build dependencies (§10.1), `cargo-deny`, `cargo-nextest`, `typst`. Windows builds happen natively, not in the container.
6. **Spike A — GPUI:** in `crates/decel`, open a window with gpui-component, a 1 000 × 50 dummy grid painted by a custom element, and a gpui-component code editor with Lua highlighting (`tree-sitter-lua` feature). Record the exact `gpui-component` and `gpui-pre` versions that work in `DESIGN.md §2 D3`.
7. **Spike B — mlua:** in `decel-engine`, create a vendored Lua 5.4 VM with the `send` feature, move it to another thread, run code with an instruction-count hook and a memory limit, and test that an infinite loop is interrupted.
8. **Spike C — install path:** `cargo install --path crates/decel` succeeds on Linux (CachyOS) **and** Windows (MSVC), and the window opens on both.
9. **`CLAUDE.md`** (template below).
10. Write README build prerequisites for Arch/CachyOS, Debian/Ubuntu, Fedora and Windows.

### `CLAUDE.md` template

```markdown
# decel — notes for Claude Code
- Spec: DESIGN.md (source of truth). Plan: IMPLEMENTATION_PLAN.md. Work one phase at a time.
- Crates: decel-model (pure types) → decel-store (SQLite) → decel-engine (Lua, deps, recalc, actor) → decel (GPUI app). decel-io: CSV.
- Only `decel` may depend on GPUI. The engine must stay headless.
- Lua: mlua lua54+vendored+send. Sandbox per DESIGN §4.5 — never expose io/os/load/require.
- Rows/cols are 1-based in Lua and UI, 0-based in Rust (`CellAddr`). Convert only at the Lua/UI boundary.
- Every mutation = one txn through decel-store; never write SQLite elsewhere.
- Before finishing a task: cargo fmt; cargo clippy --workspace --all-targets -D warnings; cargo nextest run --workspace; cargo deny check.
- No git dependencies. No unwrap/expect outside tests.
- If you change behaviour described in DESIGN.md, update DESIGN.md in the same change.
```

### Acceptance criteria

- [ ] CI is green on Linux. The Windows job is green, or documented as pending a runner, with Spike C done manually on Windows.
- [ ] `cargo publish --workspace --dry-run` passes. There are no git dependencies.
- [ ] The spike window renders the dummy grid and highlights Lua in the editor on Linux and Windows.
- [ ] The mlua test interrupts `while true do end` within 100 ms and survives `string.rep("x", 1e10)` with a memory error rather than a crash.
- [ ] The pinned GPUI versions are recorded in DESIGN.md.

**Contingency:** if gpui-component can't be published-installed, switch to plain `gpui` from crates.io, add "build minimal code editor widget" to Phase 5, and update D3.

---

## Phase 1 — Model & store

**Goal:** a correct, persistent, append-only data layer with undo/redo and history. No Lua yet.

### Tasks

1. **`decel-model`:** `CellAddr { sheet: SheetId, row: u32, col: u32 }` (0-based), `Value` and `CellError` (§3.2), `Cell { raw: Option<Value>, meta: Option<MetaHash> }`, `MetaSource` (ordered members: name, `MemberKind::{Field, Hook, Helper}`, source text), canonicalisation and normalisation (§4.2), `MetaHash` (BLAKE3, 16 bytes), sparse `Sheet` storage (`BTreeMap<(row, col), Cell>` behind a trait so it can be replaced later).
2. **Ops:** `SetCells(Vec<(CellAddr, Cell)>)`, `AddSheet`, `RenameSheet`, `MoveSheet`, `DeleteSheet`, `SetLayout`, `ChartUpsert/Delete`. Structural ops are stubbed here and finished in Phase 7.
3. **Default value parser** (§3.3) in the model crate, with exhaustive tests.
4. **`decel-store`:** schema v1 exactly as in §7.2, a migration framework keyed on `user_version`, `application_id` check, WAL mode, lock file (§7.1), read-only fallback.
5. **Transaction apply:** compute before-images, write `txns`, `cell_changes`, `other_changes` and the current-state tables in one SQLite transaction, and insert any new `metatables` rows.
6. **Undo/redo/restore** (§7.3). Rebuild the stack from `txns` on open.
7. **Queries:** load workbook, `cell_history(addr)`, `share_set(hash) -> Vec<CellAddr>`, `metatable(hash)`.
8. **Save As** via `VACUUM INTO`. New workbook creates `Sheet1`.

### Acceptance criteria

- [ ] Property test: a random sequence of ≤ 200 ops followed by undo-all gives the initial state, and redo-all gives the final state.
- [ ] Property test: close and reopen the file at a random point, and the materialised state and undo stack are unchanged.
- [ ] Property test: applying the `cell_changes` "after" images in txn order reproduces the `cells` table.
- [ ] Identical metatable source from two code paths gives the same hash. A whitespace-only trailing difference gives the same hash. Any other change gives a different hash.
- [ ] A 10 k-cell `SetCells` commits in under 100 ms, and its undo also takes under 100 ms (criterion bench, release).
- [ ] Opening a file that's locked by another process returns `StoreError::Locked { pid, host }`.

---

## Phase 2 — Lua engine

**Goal:** headless evaluation of metatables with the full `c` API, correct incremental recalculation, and safe failure.

### Tasks

1. **Sandbox** (§4.5): build the restricted globals, remove the listed functions, redirect `print` to a per-evaluation log buffer, apply the instruction hook (configurable budget) and memory limit.
2. **Stdlib:** `fmt`, `parse`, `color`, `range` modules (§4.5), written in Rust where precision matters (dms, si, thousands) or in embedded Lua otherwise. Golden tests for every formatter, including negative, zero, NaN, ±inf and rounding carries (59.99995″).
3. **Compile cache:** `MetaHash → RegistryKey` of the frozen table. Compile per member first to report errors per member and line. Generate the chunk (§4.2). Freeze with a proxy whose `__newindex` errors.
4. **Handles as userdata:** `CellCtx` (`c`), `CellHandle`, `RangeHandle`, `SheetHandle` with exactly the API in §4.3. All reads go through a `DepRecorder` active for the current evaluation.
5. **Dependency graph:** forward and reverse edges, a per-sheet `rstar` R-tree for range dependencies, name dependencies, display-graph separation (§6.1). Dependencies are replaced on each evaluation.
6. **Recalculation** (§6.2): dirty marking, demand-driven evaluation with an in-progress stack for cycle detection, error propagation (§6.3), sliced execution with a pre-emption check callback.
7. **Pipelines:** `enter_text(addr, text) -> Result<Cell, EditRejected>` (parse then validate), `final_value(addr)`, `display(addr) -> DisplayCell`, `tooltip(addr)`, `edit_text(addr)`, `edit_hint(addr)`, `preview(addr, MetaSource)`.
8. A **headless test harness** that builds workbooks from a small Rust DSL and asserts values, for example `wb.set(1,1, 10).meta(1,2, running_total())…`.

### Acceptance criteria

- [ ] The examples in §4.7 (latitude) and §4.8 (running total, filled 500 rows) behave as specified, including parse rejection, validation messages and tooltips.
- [ ] A cycle gives `#CYCLE` on every member of the cycle and nowhere else. Breaking the cycle recovers all of them.
- [ ] `while true do end` in `value` gives `#TIMEOUT` without hurting other cells. The memory bomb gives `#MEMORY`.
- [ ] Sandbox escape tests: `io`, `os`, `load`, `require`, `debug`, `string.dump`, `getmetatable("").__index` mutation and handle metatable access all fail or are harmless.
- [ ] **Differential fuzz test:** random workbooks with random edit sequences give identical final values under incremental recalculation and full recalculation from scratch.
- [ ] A range dependency picks up a newly populated cell inside the range (for example `c:above():sum()` updates when a blank above is filled).
- [ ] Bench: one edit with 1 k dependents under 16 ms. Full recalc of 100 k cells with a simple `value` under 1 s (release).

---

## Phase 3 — Engine actor

**Goal:** the engine runs on its own thread behind a typed protocol. This is everything the UI will use.

### Tasks

1. **`Command` enum:** `Open`, `New`, `SaveAs`, `EnterText`, `SetCells`, `SetMeta { targets, source, mode: Cow | Shared }`, `Fill { kind, src, dst }`, `AutoFill { addr, axis }`, `Paste { mode }`, `Undo`, `Redo`, `Restore`, `Subscribe { sheet, rect }`, `Preview`, `QueryHistory`, `QueryShareSet`, plus sheet, layout and chart ops (stubs where later phases fill them in). Each carries `seq: u64`.
2. **`Event` enum:** `Ack { seq, result }`, `CellsUpdated { sheet, cells: Vec<(CellAddr, DisplayCell)> }`, `TxnCommitted { label, can_undo, can_redo }`, `RecalcProgress { pending }`, `HistoryResult`, `ShareSetResult`, `Error`.
3. **Actor loop:** handle commands, recalc in about 5 ms slices with pre-emption by new commands, push `CellsUpdated` only for subscribed rectangles plus a margin, and coalesce updates.
4. **Fill semantics** (§5.2) and **auto-fill run detection**, as engine operations with a "preview" variant that returns the affected addresses without committing.
5. **COW vs shared metatable edits** (§5.3) as engine operations returning the affected addresses.
6. **`decel-cli`** dev binary (`publish = false`): `decel-cli eval file.decel --sheet S --dump csv` for end-to-end tests and debugging.

### Acceptance criteria

- [ ] An integration test drives the actor only through channels: open, edit, fill 500, undo and redo, with values asserted via `CellsUpdated`.
- [ ] A pre-emption test: start a 100 k-cell recalc, send an edit, and the edit's `Ack` arrives within 20 ms.
- [ ] Auto-fill run detection has table-driven tests covering a blank stop, a hash change stop, a run of "no meta" cells with values, and selection at the end of a run.
- [ ] A COW edit changes exactly one cell's hash. A shared edit changes every cell in the share set across sheets, in one txn that is undone as one step.

---

## Phase 4 — Grid UI

**Goal:** a usable spreadsheet window for values, with no metatable tooling yet.

### Tasks

1. App shell: window, client-side title bar, theme (light/dark following the OS), bottom sheet tabs, status bar, edit bar (§8.1).
2. **Grid element** (§8.2): virtualised layout with a prefix-sum structure for sizes, frozen headers, painting order, text shaping cache, overflow spill, error and warning corner markers.
3. Selection: single cell, rectangle, whole row and column; keyboard navigation (arrows, `Ctrl+arrows` to data edges, PgUp/PgDn, Home/End); mouse drag selection; auto-scroll.
4. Editing (§8.5): type-to-edit, `F2` and double-click with `edit()` text, edit hint, commit and cancel keys, rejection display, optimistic display with reconciliation by `seq`.
5. Hover tooltips (§8.5).
6. Column resize by dragging, double-click auto-fit, row resize.
7. Files: New, Open, Save As, a recent-files list, a window title with the file name, the read-only banner on lock, and opening a file passed on the command line (`decel path.decel`).
8. Undo/redo keys wired to the engine (the panel comes in Phase 6).

### Acceptance criteria

- [ ] With 1 M rows × 20 columns of numbers, scrolling holds 120 fps on the desktop (use a frame-time overlay behind `DECEL_DEBUG_FPS=1`).
- [ ] Keypress-to-paint latency under 8 ms (measured with the overlay).
- [ ] The latitude example from §4.7, created through the test harness, displays as DMS, edits as decimal, rejects 95, and shows the tooltip.
- [ ] Killing the process mid-session loses at most the in-progress edit. Reopening shows every committed edit, and undo still works.
- [ ] Works on Wayland, X11 and Windows. Manual check documented in `docs/manual-tests.md`.

---

## Phase 5 — Metatable UX

**Goal:** the features that make decel decel.

### Tasks

1. **Colour assignment** (§8.3): 16-hue OKLCH palettes for light and dark, first-appearance assignment persisted in workbook settings, hatch beyond 16.
2. **Peek quasimode:** hold-key detection (key-down and key-up, plus losing focus releases it), tint overlay, floating legend (kind, unit, count, colour swatch).
3. **Automatic boundaries:** draw them when hashes differ (§8.3), with the `Ctrl+Shift+B` toggle.
4. **Code side:** an `F3` toggle with a ~150 ms flip animation (scale-X through 0), cells show `kind` or the first member name, emphasised boundaries, and the Metatable dock opens.
5. **Metatable dock** (§8.4): member list with status, + add hook and + helper, fixed signature header, body editor (gpui-component code editor with Lua highlighting), per-member compile status, live preview against the selected cell, `print` log, commit on blur or `Ctrl+Enter`, revert with `Esc`.
6. **COW and shared edit** (§5.3): banner text, `Shift`-hover share-set outline with a count (including other sheets), `Shift+click` for a shared edit, and a Detach command.
7. **Fill handle drag**, fill down/right, auto-fill column/row with a preview outline before confirming (§5.2).
8. **Clipboard:** an internal clipboard of `(raw, hash)` rectangles; external clipboard as TSV of edit text; paste, paste values and paste meta.
9. Assign a metatable to a selection by pasting meta, or with "New metatable on selection" from the palette (creates an empty meta shared by the selection).

### Acceptance criteria

- [ ] Walkthrough test in `docs/manual-tests.md`: build the §4.7 and §4.8 examples entirely through the UI without the test harness.
- [ ] Hold-peek shows colours within one frame of key-down and clears within one frame of key-up. Alt-tabbing away while holding doesn't leave it stuck.
- [ ] A COW edit on one of 500 filled cells changes that cell's colour only. A Shift+click edit changes all 500 cells' behaviour, and their colour stays shared.
- [ ] Auto-fill column replaces exactly the previewed run, and undo restores it in one step.
- [ ] The Lua editor never loses typed text: switching members, toggling the code side and window focus changes all keep uncommitted text per member until you commit or revert.

---

## Phase 6 — History UI

**Goal:** make the stored history visible and useful.

### Tasks

1. **History dock** (`F5`) for the selected cell: a list of txns (time, label, before → after display, a metatable-changed badge) using `QueryHistory`.
2. **Member-level diff** between two metatable versions, shown side by side per member, with added, removed and changed members flagged.
3. **Restore** a historical version (a `kind=restore` txn).
4. An **Undo history** palette view: a list of recent txns with the undo/redo position, where clicking jumps by undoing or redoing to that point.
5. Status bar: undo and redo availability, plus the label of the last txn.

### Acceptance criteria

- [ ] A cell edited 50 times over 3 sessions shows all 50 versions in order. The query takes under 10 ms with 1 M `cell_changes` rows (seeded bench).
- [ ] Restore, then undo, returns the cell to its pre-restore state.
- [ ] The diff correctly shows a renamed helper as a removal plus an addition, and a body edit as a change.

---

## Phase 7 — Sheets & structure

**Goal:** multiple sheets and row/column structure editing.

### Tasks

1. Sheet tabs: add, rename (inline), reorder by dragging, delete with confirmation. All are transactions that can be undone.
2. `c:sheet_named()` with name dependencies, so a rename invalidates correctly (§6.1).
3. **Insert and delete rows and columns:** a structural op in the store (§7.2), shifting cells, layout, chart anchors and the dependency index. Relative offsets keep working. Absolute `c:at()` calls are **not** rewritten (documented in the user guide).
4. History coordinate mapping through structural ops (§7.4).
5. The share-set highlight and colours work across sheets (legend counts per sheet).

### Acceptance criteria

- [ ] Inserting 3 rows inside a 500-row running total keeps every total correct. The new blank rows are treated as "no value" by `c:up()` logic.
- [ ] Deleting a row that a range dependency references recalculates its dependents correctly.
- [ ] Renaming the sheet "Inputs" to "Params" makes cells that look up `"Inputs"` show a `Ref` error, and renaming it back recovers them.
- [ ] Per-cell history follows a cell that moved down through two row inserts.
- [ ] Property test: random structural ops mixed with edits, then undo-all, gives the initial state.

---

## Phase 8 — CSV import/export

### Tasks

1. **Import:** to a new sheet or at the selection. Options: delimiter (auto-detect), header row, encoding (UTF-8 with or without BOM, plus Windows-1252 fallback). Values go through the default parser, or through the target cells' `parse` hooks when importing over cells that already have metatables. One txn.
2. **Export:** a selection or sheet, as either the **displayed text** or the **edit text/raw values**, with configurable delimiter and quoting.
3. Palette commands, plus drag-and-drop of a `.csv` onto the window.

### Acceptance criteria

- [ ] Round trip: export raw, then import, gives identical raw values for ints, floats (shortest-round-trip formatting), bools and strings with quotes, commas and newlines.
- [ ] A 100 k-row import takes under 2 s and is undone in one step.

---

## Phase 9 — Charts

### Tasks

1. A chart spec type (JSON-serialisable), plus store ops and history (§8.6).
2. A renderer drawn with GPUI paths: axes, "nice" tick generation, gridlines, line, bar (grouped) and scatter series, a legend and a title. Values that aren't numbers are skipped, and the count of skipped values is shown in a corner note.
3. A floating chart element: anchored to a cell, move and resize with the mouse, selection outline, delete key.
4. The chart panel in the dock: type, title, X range, series list (range, name, colour), axis titles and the legend toggle. Ranges are picked by selecting on the grid.
5. Live updates on `CellsUpdated` for the chart's source ranges. Charts subscribe to their ranges even when off-screen.

### Acceptance criteria

- [ ] Line, bar and scatter charts of the §4.8 running total update live while values are edited.
- [ ] A chart stays anchored correctly after row and column inserts above or left of it.
- [ ] A chart edit can be undone. Charts survive save, reopen and Save As.

---

## Phase 10 — Polish & release 1.0.0

### Tasks

1. **Command palette** with fuzzy matching over every command, showing keybindings.
2. **Config** (§8.7): load and hot-reload `config.toml` for keymap, theme, font, peek key, budget and memory cap. Invalid config gives a non-fatal notification.
3. **Performance pass** against every target in §11. Fix regressions and add criterion benches to CI as a non-blocking report.
4. **Robustness:** fuzz the file loader (`cargo-fuzz` on schema-valid but hostile files), check that no panics are reachable, and add a panic hook that writes a crash log and keeps the file intact.
5. **Docs:** `docs/user-guide.md` (concepts, modes, fill/COW, keymap), `docs/lua-api.md` (the full §4 reference with examples), and `examples/*.decel` (survey points with lat/lon, budget with currency, an engineering calc with unit tags).
6. **Release prep:** crate metadata (description, keywords, categories, readme), a `--version` flag, `CHANGELOG.md`, and publishing the crates in dependency order (`cargo publish --workspace`), then verifying `cargo install decel` on clean Linux and Windows machines.

### Acceptance criteria (= v1.0 definition of done)

- [ ] Every §11 target is met, or a documented exception is accepted by Nicholas.
- [ ] `cargo install decel` from crates.io works on a clean CachyOS install and a clean Windows 11 install with only the documented prerequisites.
- [ ] Every checklist in `docs/manual-tests.md` passes on both OSes.
- [ ] The engine and store have ≥ 85 % coverage (`cargo llvm-cov`). All property and fuzz tests pass.
- [ ] Every crate is MIT OR Apache-2.0, and `cargo deny check` is clean.

---

## Suggested Claude Code workflow per phase

```text
1. Plan mode (Shift+Tab) → propose task breakdown for Phase N; Nicholas reviews.
2. Implement in small commits; run the full check suite after each task.
3. For engine/store work: write the failing test first.
4. For UI work: add/extend docs/manual-tests.md with the steps to verify, and ask Nicholas to run them on Windows.
5. Update CHANGELOG.md and any DESIGN.md decisions; tag phase-N.
```

Good places to use subagents: Phase 2's stdlib formatters and golden tests, Phase 8 and Phase 9 in parallel worktrees, and Phase 10's docs and examples while the performance pass runs.
