# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
