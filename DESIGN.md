# decel — Design Document

| | |
|---|---|
| **Status** | Draft for v1.0 |
| **Date** | 2026-11-02 |
| **Author** | Nicholas (with Claude) |
| **Licence** | MIT OR Apache-2.0 |
| **Install** | `cargo install decel` |

---

## 1. Purpose

decel is a lightweight desktop spreadsheet in which every cell can carry a **Lua metatable**. The metatable decides how the cell's value is computed, displayed, edited, parsed, validated and described. Formatting, number formats, validation rules and formulas, which Excel spreads across many dialogs, become one programmable object per cell. You can see that object, colour cells by it, share it between cells and track its history.

### 1.1 Goals

1. **Programmable cells.** Any cell can hold an optional raw value plus an optional metatable written in Lua 5.4.
2. **Make cell kinds visible.** Hold a key to colour cells by metatable. Draw boundaries automatically wherever the metatable changes. Flip the grid to its "code side" to edit metatables in place.
3. **Never lose work.** Every change is stored in a SQLite workbook file with full, non-destructive history. Undo persists across sessions, and every cell has its own history.
4. **Keep the UI minimal and fast.** Desktop-first, keyboard-driven and GPU-rendered (GPUI). There is no ribbon, and every command lives in a palette.
5. **Function-at-a-time code editing.** As in REALbasic, a metatable is shown as a list of members, and each member is edited on its own.
6. **Easy to install.** `cargo install decel` on Linux and Windows, with no system Lua or SQLite needed.

### 1.2 Non-goals for v1.0

- Web or WASM build. The engine is kept UI-independent so a web build stays possible later.
- macOS as a supported target. It may well work, but it isn't tested or promised.
- Excel formula compatibility and XLSX import/export.
- Unit conversion or dimensional analysis. Units are tags only (§4.6).
- Real-time multi-user collaboration.
- Glass or translucent window effects.
- Workbook-level shared Lua modules (§13 lists this as a candidate).

---

## 2. Decisions from the design interview

| # | Topic | Decision | Notes |
|---|---|---|---|
| D1 | Name | **decel**, crate `decel`, file extension `.decel` | `dcel` is already taken on crates.io, and the name clashes with the doubly-connected-edge-list data structure. `decel` is free as of 2026-11-02. |
| D2 | Platform priority | Desktop first. Web is secondary and may come later. | |
| D3 | UI framework | **GPUI**, plus **gpui-component** for widgets and the code editor | Pinned in Phase 0 (2026-11-02): `gpui-component =0.9.0` (with `tree-sitter-lua`), `gpui-pre =0.3.11` and `gpui-pre-platform =0.3.11` (features `font-kit`, `wayland`, `x11`). gpui-component pins `gpui-pre` exactly, so decel must use the same version (§10.2). Since gpui-pre 0.3.11 the app is started with `gpui_platform::application()`, which lives in the separate `gpui-pre-platform` crate. |
| D4 | Scripting | **Lua 5.4** via `mlua` with the `vendored` feature | Builds from source, so no system Lua is needed. |
| D5 | Cell model | A cell has an **optional raw value** and an **optional metatable**. The metatable can override computation, display, editing text, parsing, validation, the hover tooltip and the editing hint. | Example: store a latitude as decimal degrees, display it as D°M′S″, and edit it as decimal. |
| D6 | Cell references | **API calls with integer rows/columns and relative offsets**, plus helpers for "this column", "this row", "above" and so on. There is no A1 syntax inside Lua. | Relative offsets let one shared metatable work in every row. |
| D7 | Metatable attachment | **Per cell.** Fill operations share one Lua object between cells. Editing is copy-on-write by default. | §5 |
| D8 | Grouping for colours and borders | **Structural hash.** Cells whose metatables have identical source share colour and identity. | Metatables are content-addressed, so a shared object and identical source are the same thing. |
| D9 | Editing a shared metatable | **Default: copy-on-write**, so only the clicked cell gets the change. **Holding the share modifier** highlights every cell that will change while you hover. Clicking then edits a copy that is applied to all of those cells. | §5.3 |
| D10 | History UI | Persistent undo/redo, plus a **per-cell history panel** | The full history is always stored. A timeline scrubber and branches are post-v1. |
| D11 | Units and currency | An **optional `unit` tag** in the metatable, with no arithmetic semantics | Conversion was rejected because it breaks down as soon as any arithmetic is done. |
| D12 | Platforms for v1.0 | **Linux** (Wayland and X11) and **Windows** | |
| D13 | Lua sandbox | **Pure sandbox**: no io, os, file `require` or network | Safe to open workbooks from other people. |
| D14 | Charts | **Floating charts configured in a panel**: line, bar and scatter | |
| D15 | Metatable capabilities | Display text and colours, alignment and font style, validation, edit parsing (plus computation, tooltips and hints) | |
| D16 | v1.0 scope extras | Multiple sheets, CSV import/export, charts | XLSX import is out. |
| D17 | Licence | MIT OR Apache-2.0 (dual) | |

---

## 3. Concepts

### 3.1 Workbook, sheet, cell

- A **workbook** is one `.decel` file, which is a SQLite database. It contains sheets, metatables, charts and history.
- A **sheet** is a sparse grid addressed by integer `(row, col)`. These are **1-based** in Lua and in the UI, and 0-based internally. Row and column headers show numbers by default. A setting switches column headers to letters, for display only.
- A **cell** is a `(raw, meta)` pair:
  - `raw: Option<Value>` is what the user typed, after parsing.
  - `meta: Option<MetaHash>` points to an immutable, content-addressed metatable.
  - A cell is **empty** if both are `None`. Empty cells are not stored.

### 3.2 Values

```text
Value = Nil | Bool(bool) | Int(i64) | Num(f64) | Str(Arc<str>) | Error(CellError)
CellError = { kind: Lua | Cycle | Timeout | Memory | Ref | Validation | Type, message, member?, line? }
```

Dates, times, money and angles are plain numbers or strings whose meaning comes from the metatable. A date is a number (days since 1970-01-01, UTC) formatted by `fmt.date`. Cells can't hold Lua tables as values in v1. They are allowed only as return values of `display` and `style`.

### 3.3 The value pipeline

```mermaid
flowchart LR
  T[typed text] --> P{meta.parse?}
  P -- yes --> PV[parse(self,c,text)]
  P -- no --> DP[default parser]
  PV --> V{meta.validate?}
  DP --> V
  V -- ok --> S[(store raw)]
  V -- fail --> E[reject edit, show message]
  S --> C{meta.value?}
  C -- yes --> CV[value(self,c)]
  C -- no --> R[raw]
  CV --> F[final value]
  R --> F
  F --> D{meta.display?}
  D -- yes --> DD[display(self,c,v)]
  D -- no --> DF[default format]
```

- **Default parser.** It tries, in order: empty text means clear the raw value; `true`/`false`; an integer; a float (`1e3`, `-0.5`, `1_000` and `1,000` are all accepted); otherwise a string. A leading `'` forces a string.
- **The final value** is used by other cells, charts and display. It is `value(self, c)` when the metatable defines `value`. Otherwise it is `raw`.
- **Validation** runs when you enter a value, against the parsed raw value. A failure rejects the edit and keeps the editor open with the message. `validate` also runs on computed values after recalculation. A failure there doesn't reject anything. It shows a **warning marker** (a corner triangle) with the message in the tooltip.
- **Display functions** (`display`, `tooltip`, `edit`, `edit_hint`) are evaluated **lazily**, only for cells that are visible or being interacted with.

---

## 4. Metatables

### 4.1 Structure

A metatable is an ordered set of **members**. Each member is either a **hook** with a fixed signature, a **field** holding a literal or expression, or a **helper** function. Members are stored and edited one at a time, which is the REALbasic-style list (§8.4).

| Member | Kind | Signature / type | Purpose |
|---|---|---|---|
| `kind` | field | string | Label shown in the legend, in peek mode and in the code side |
| `unit` | field | string | Unit or currency tag (`"m"`, `"deg"`, `"AUD"`). Display and grouping only. |
| `style` | field | table | Static style defaults (§4.4) |
| `value` | hook | `value(self, c) -> value` | Computed value |
| `display` | hook | `display(self, c, v) -> string \| DisplayTable` | Rendered text and style |
| `edit` | hook | `edit(self, c, v) -> string` | Text placed in the editor when editing starts |
| `parse` | hook | `parse(self, c, text) -> value \| (nil, err)` | Turns typed text into the raw value |
| `validate` | hook | `validate(self, c, v) -> true \| (false, msg)` | Accept or reject a value |
| `tooltip` | hook | `tooltip(self, c, v) -> string?` | Text shown on hover |
| `edit_hint` | hook | `edit_hint(self, c) -> string?` | Hint shown in the edit bar while editing |
| *other* | helper | any Lua expression, usually `function(self, ...)` | Private helpers, called as `self:name(...)` |

The editor shows each hook with its signature fixed and lets you edit only the body. Helpers are free-form expressions.

### 4.2 Canonical form and hashing

A metatable's canonical source is its members, sorted by name and serialised as:

```text
decel-meta/1
<name>\t<kind>\t<len>\n<source bytes>\n
...
```

`MetaHash = BLAKE3(canonical)`, truncated to 16 bytes and stored as a BLOB. Source is normalised before hashing: line endings become `\n`, trailing whitespace is trimmed, and a trailing newline is added. Two cells with identical metatable source therefore always share a hash. This gives the **structural identity** used for colouring, borders, sharing and storage deduplication.

The compiled Lua chunk is generated as:

```lua
local M = {}
M.kind = <expr>
function M.value(self, c) <body> end
...
return M
```

Each member is compiled separately first, so syntax errors are reported against that member and line.

### 4.3 The cell context `c`

Every hook receives `c`, a lightweight handle to the cell being evaluated. All reads through `c` are **recorded as dependencies** (§6).

```lua
-- identity
c.row, c.col           -- 1-based integers
c.sheet                -- Sheet handle for this cell's sheet
c.raw                  -- this cell's raw value (no dependency; it's this cell's own input)
c.kind, c.unit         -- from this cell's metatable (or nil)

-- single cells → Cell handles
c:offset(dr, dc)       -- relative: c:offset(0, -1) is the cell to the left
c:at(row, col)         -- absolute, on this sheet
c:up(n), c:down(n), c:left(n), c:right(n)    -- n defaults to 1

-- ranges → Range handles
c:above()              -- this column, rows 1 .. row-1
c:below()              -- this column, rows row+1 .. last used row
c:left_of()            -- this row, cols 1 .. col-1
c:right_of()           -- this row, cols col+1 .. last used col
c:column(r1, r2)       -- this column; r1/r2 default to 1 / last used, excluding self
c:row_range(c1, c2)    -- this row; defaults like column(), excluding self
c:rect(dr1, dc1, dr2, dc2)   -- relative rectangle

-- other sheets
c:sheet_named("Inputs")      -- Sheet handle (dependency on the name)
sheet:at(row, col)  sheet:range(r1, c1, r2, c2)  sheet.name  sheet:last_row()  sheet:last_col()
```

**Cell handle**

```lua
h.value      -- final value of that cell (evaluates it if dirty; raises if it holds an error)
h.raw        -- raw value
h.ok         -- false if the cell holds an error
h.err        -- error message or nil
h.kind, h.unit, h.row, h.col
h:offset(dr, dc)
```

**Range handle**

```lua
r:values()          -- array of final values, nils skipped
r:cells()           -- iterator of Cell handles
r:sum() r:min() r:max() r:avg() r:count()   -- numeric; errors propagate
r:map(f)  r:filter(f)  r:reduce(f, init)
r:where_kind("latitude")   -- cells whose meta.kind matches
r.rows, r.cols
```

Ranges are recorded as **range dependencies**, not as one dependency per cell, so `c:above():sum()` stays cheap and also picks up cells added later.

**Excluding self.** The helpers whose names describe "the rest of this column or row" (`column`, `row_range`, `above`, `below`, `left_of`, `right_of`) never include the evaluating cell. A "Total" cell at the bottom of a column can therefore use `c:column():sum()` without creating a cycle.

### 4.4 Display tables and style

`display` may return a string, or a table such as:

```lua
{ text = "35°18′29″S", fg = "#c0392b", bg = color.rgb(255, 240, 240),
  align = "right",   -- "left" | "centre" | "right"
  bold = false, italic = false, mono = true }
```

`style` is a static table with the same keys except `text`. The precedence is: the display table, then `style`, then the default (numbers right-aligned in a proportional font, text left-aligned). Colours are `#rrggbb` strings or values from the `color.*` helpers.

### 4.5 The standard library inside the sandbox

The Lua 5.4 libraries available are: `math` (without `random`/`randomseed`), `string`, `table` and `utf8`. The base functions `load`, `loadstring`, `dofile`, `loadfile`, `require`, `collectgarbage`, `rawset` and `setmetatable` are removed. `print` is redirected to the per-cell log shown in the editor.

`pcall` and `xpcall` are replaced by thin wrappers around the native functions. The wrappers re-raise timeout and memory-limit errors, so a script can't catch its own interrupt and keep looping. Ordinary errors are still caught and keep their original error values. A `string.rep`/concatenation request larger than Lua's internal limit (`INT_MAX`) is also reported as `#MEMORY`.

decel adds:

| Module | Functions |
|---|---|
| `fmt` | `fixed(v, dp)`, `thousands(v, dp)`, `percent(v, dp)`, `sci(v, sig)`, `si(v, unit, sig)` (engineering prefixes, display only), `currency(v, symbol, dp)`, `dms(v, "lat"\|"lon"\|nil, sec_dp)`, `date(v, pattern)`, `duration(seconds)` |
| `parse` | `number(text)`, `dms(text)`, `date(text, pattern)`, `currency(text)` |
| `color` | `rgb(r, g, b)`, `hex(s)`, `lerp(a, b, t)`, `scale(v, lo, hi, a, b)` (for heat-map backgrounds) |
| `range` | `sum(...)`, `min(...)`, `max(...)`: variadic helpers that accept values or ranges |

Implementation notes (Phase 2):

- The global environment is a frozen, read-only table. Assigning a global is an error ("use local"), and `string`, `math`, `table`, `utf8` and decel's modules are read-only proxies, so no cell can leave state behind for another. `getmetatable` is kept, but strings report `false` and every frozen table reports `"locked"`. Compiled metatables are frozen the same way.
- `range.sum`/`min`/`max` and the Range methods skip non-numbers. `sum` stays an integer while every term is one. `min`/`max`/`avg` return floats, and `nil` for an empty range. `count` counts non-nil values.
- `r:map(f)` and `r:filter(f)` call `f` with Cell handles. `r:filter` returns an array of handles. `r:reduce(f, init)` folds over values (`f(acc, v)`). `r.rows`/`r.cols` of an open-ended range (`c:below()`, `c:right_of()`, `c:column()` with no end) stop at the last populated cell inside the range.
- `display`, `edit` and `tooltip` hooks aren't called when the value is `nil`. An empty cell with a metatable shows nothing.
- `fmt` rounds half to even on the exact binary value (`fmt.fixed(1.005, 2)` is `"1.00"`). `fmt.dms` rounds half away from zero and carries 60″ into minutes and degrees. Every parser accepts `−` (U+2212) as a minus sign. The full list of formatter edge cases is in the `stdlib` module docs and tests.

Removing `math.random` and all time functions keeps evaluation **deterministic**. Recalculation therefore gives the same result for the same inputs, which keeps history meaningful.

### 4.6 Units

`unit` is metadata only. It appears in:

- the legend and the peek/code-side overlays,
- the default tooltip (`12.5 m`),
- `c.unit` and `h.unit`, so a metatable can make its own decisions, such as showing a warning if summed cells have mixed units.

decel never converts or checks units in arithmetic. `fmt.si` chooses an engineering prefix for display (`12500` with `"m"` becomes `12.5 km`). That is formatting, not conversion.

### 4.7 Example: latitude as D°M′S″, edited as decimal

| Member | Body |
|---|---|
| `kind` | `"latitude"` |
| `unit` | `"deg"` |
| `display` | `return fmt.dms(v, "lat", 1)` |
| `edit` | `return string.format("%.6f", v)` |
| `parse` | `local n = tonumber(text) or parse.dms(text)`<br>`if not n then return nil, "Enter decimal degrees or D°M'S\" N/S" end`<br>`return n` |
| `validate` | `if type(v) ~= "number" or v < -90 or v > 90 then return false, "Latitude must be within ±90°" end`<br>`return true` |
| `tooltip` | `return string.format("%.6f° — %s", v, fmt.dms(v, "lat", 2))` |
| `edit_hint` | `return "Decimal degrees (−35.3075) or 35°18'27\"S"` |

### 4.8 Example: running total, shared by filling down

| Member | Body |
|---|---|
| `kind` | `"running total"` |
| `value` | `local prev = c:up()`<br>`local base = (prev.row >= 1 and prev.kind == self.kind) and prev.value or 0`<br>`return base + c:left().value` |
| `style` | `{ align = "right", mono = true }` |

Fill this metatable down 500 rows and every cell shares one Lua object, one hash and one colour. It still works in every row because it only uses relative offsets. (`c:up()` above row 1 returns a handle with `row = 0`, whose `.value` is `nil`.)

---

## 5. Sharing, fill and copy-on-write

### 5.1 Model

Cells refer to metatables by `MetaHash`. The engine keeps **one compiled Lua table per hash** (the compile cache). Cells that share a hash share a Lua object, and that sharing is exactly what "filled cells reference one object" means. Compiled tables are **frozen**: writing to them raises an error. Per-cell state therefore has to come from `c`, never from fields on the shared table.

### 5.2 Fill operations

| Operation | Gesture | Effect |
|---|---|---|
| **Fill drag** | Drag the fill handle at the selection corner | Copies the selection's metatables (and, with the value-modifier held, its raw values) across the dragged range, repeating the pattern. One transaction. |
| **Fill down / right** | `Ctrl+D` / `Ctrl+R` | The same, from the top or left cell of the selection |
| **Auto-fill column** | `Ctrl+Shift+D` | From the selected cell, find the **adjacent run**: the maximal contiguous block above and below in the same column whose cells are non-empty and share one hash (the hash of the immediate neighbour, which may be "no metatable"). Set all of them to the selected cell's metatable. The run is previewed before you confirm. |
| **Auto-fill row** | `Ctrl+Shift+R` | The same along the row |
| **Paste** | `Ctrl+V` | Pastes the raw values and hashes. Pasted cells share metatables with the source. |
| **Paste values** | `Ctrl+Shift+V` | Raw values only. Existing metatables stay. |
| **Paste meta** | `Ctrl+Alt+V` | Metatables only |

### 5.3 Editing a shared metatable (D9)

The **share set** of a cell is every cell in the workbook with the same hash.

- **Plain click** on a cell in the code side opens its metatable. Edits are **copy-on-write**: when you commit, a new hash is created and **only that cell** is repointed to it. Before the first keystroke, the editor header shows "Editing this cell only · shared by 312 others".
- **Hold the share modifier** (default `Shift`) in the code side. Hovering a cell highlights its whole share set with an outline and a count, including a count of cells on other sheets. **Shift+click** opens a **shared edit**. When you commit, every cell in the share set is repointed to the new hash in one transaction. The header shows "Editing 313 cells (3 on other sheets)".
- **Detach** (a palette command) forks the selected cells onto a private copy without changing the source. This exists so you can later edit them as their own group.
- Because identity is structural, two cells that were typed separately with identical code are in the same share set. The hover highlight always shows exactly what a shared edit will touch.

Commits happen **when you leave a member editor or press `Ctrl+Enter`**, not on every keystroke. While you're typing, the compile status is shown live, but nothing is recalculated until you commit.

---

## 6. Evaluation engine

### 6.1 Dependencies are tracked at runtime

There's no formula parser, so dependencies are **recorded while code runs**:

- While cell X's `value` hook runs, every `h.value` or `h.raw` read of cell Y records the edge `Y → X`.
- Every Range method records a **range dependency** `(sheet, rect) → X`. Range dependencies live in an R-tree (`rstar`) per sheet, so finding the dependents of a changed cell is a point query.
- `c:sheet_named(name)` records a **name dependency**, so renaming a sheet invalidates the cells that looked it up by name.
- Dependencies are **replaced** on every evaluation, which handles conditional reads correctly.
- Display hooks record dependencies into a separate **display graph**. Changes there only invalidate rendering, never values.

### 6.2 Recalculation

1. An edit transaction changes a set of cells.
2. **Mark dirty.** Do a breadth-first walk over the direct edges plus range and name hits, transitively.
3. **Evaluate on demand.** Evaluate each dirty cell. Reading a dirty dependency evaluates it first, recursively. An "in progress" stack detects cycles: every cell in the cycle gets `#CYCLE`.
4. Each evaluation has an **instruction budget** (default 10 M VM instructions, enforced by a Lua hook) giving `#TIMEOUT`, and the VM has a **memory cap** (default 512 MiB) giving `#MEMORY`.
5. Changed final values are sent to the UI for any subscribed (visible) cells.

Evaluation never recurses through Lua. When a hook reads a dependency that hasn't been evaluated yet, the hook is interrupted (an interrupt `pcall` can't catch), the dependency is evaluated from an explicit work stack, and the hook is retried. Long chains such as a 500-row running total therefore never touch Lua's C-stack limit. A cycle is detected when the dependency is already on the stack, and exactly the stack slice from it to the top gets `#CYCLE`. A cell that reads a cycle member sees the error propagated (with its kind) but isn't itself marked as a member. Dirty cells are processed in row-major order, so top-down chains rarely need a retry.

Recalculation runs on the engine thread (§9) in slices of about 5 ms, and checks for new commands between slices. A newer edit **pre-empts** a running recalculation, marks more cells dirty and resumes. The UI never blocks.

### 6.3 Errors

A Lua error in a hook becomes `Value::Error` carrying the member name and line. Reading `h.value` of an error cell **re-raises** the error, so errors propagate like Excel's `#REF!` chains. Code that wants to handle an error checks `h.ok` first. Errors in display hooks fall back to the default display with a red corner marker. They never break the grid.

### 6.4 Opening a workbook

On open, the engine loads the current state, compiles every distinct metatable once, then evaluates every cell that has a `value` hook (no persisted cache in v1). Target: 100 k computed cells in under 1 s (§11).

---

## 7. Persistence

### 7.1 Principles

- **The file is always saved.** Every transaction is committed straight to SQLite (WAL mode). "Save" doesn't exist. "Save As" is `VACUUM INTO`.
- **History is append-only.** Nothing is ever deleted. Undo is itself a new transaction that restores earlier state (the Emacs/Grist model), so the full history survives undo, redo and branching edits.
- **The current state is materialised**, so opening a file never replays history.
- **One writer.** The file is opened with an advisory lock file (`<name>.decel.lock`, with PID and hostname). A second instance offers to open the file read-only.

### 7.2 Schema (v1)

```sql
PRAGMA application_id = 0x6465636C;          -- 'decl'
PRAGMA user_version   = 1;                   -- schema version; migrations bump it
PRAGMA journal_mode   = WAL;

CREATE TABLE workbook   (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- created, app_version, settings
CREATE TABLE sheets     (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, position INTEGER NOT NULL);

CREATE TABLE metatables (hash BLOB PRIMARY KEY, canonical TEXT NOT NULL, created_txn INTEGER NOT NULL)
  WITHOUT ROWID;                                                         -- immutable, never deleted

CREATE TABLE cells (
  sheet_id INTEGER NOT NULL, row INTEGER NOT NULL, col INTEGER NOT NULL,
  vtype INTEGER,            -- NULL = no raw value; 1 bool 2 int 3 num 4 str
  value,                    -- SQLite dynamic typing holds the raw value
  meta BLOB REFERENCES metatables(hash),
  PRIMARY KEY (sheet_id, row, col)
) WITHOUT ROWID;
CREATE INDEX cells_by_meta ON cells(meta);  -- share-set queries

CREATE TABLE layout (sheet_id INTEGER, axis INTEGER, idx INTEGER, size REAL,
                     PRIMARY KEY (sheet_id, axis, idx)) WITHOUT ROWID;    -- column widths, row heights
CREATE TABLE charts (id INTEGER PRIMARY KEY, sheet_id INTEGER NOT NULL, spec TEXT NOT NULL);  -- JSON

-- history
CREATE TABLE txns (
  id INTEGER PRIMARY KEY, ts INTEGER NOT NULL,       -- unix ms
  label TEXT NOT NULL,                               -- "Fill down 500 cells", "Edit latitude"
  kind INTEGER NOT NULL,                             -- 0 normal, 1 undo, 2 redo, 3 restore
  target INTEGER REFERENCES txns(id)                 -- the txn undone/redone/restored
);
CREATE TABLE cell_changes (
  txn INTEGER NOT NULL, sheet_id INTEGER NOT NULL, row INTEGER NOT NULL, col INTEGER NOT NULL,
  b_vtype INTEGER, b_value, b_meta BLOB,             -- before
  a_vtype INTEGER, a_value, a_meta BLOB,             -- after
  PRIMARY KEY (sheet_id, row, col, txn)
) WITHOUT ROWID;
CREATE INDEX cell_changes_by_txn ON cell_changes(txn);
CREATE TABLE other_changes (                         -- sheets, layout, charts, structure ops
  txn INTEGER NOT NULL, seq INTEGER NOT NULL, entity TEXT NOT NULL,
  before TEXT, after TEXT,                           -- JSON
  PRIMARY KEY (txn, seq)
) WITHOUT ROWID;
```

Implementation notes (Phase 1):

- `workbook` keys: `created`, `app_version`, the monotonic id counters `next_sheet_id` and `next_chart_id` (ids are never reused, so undo can recreate a deleted sheet or chart under its old id), and user settings as `settings.<key>`. Settings are written outside the undo history.
- `charts.spec` holds the whole chart record as JSON: the anchor cell (`row`, `col`) plus the chart's own spec.
- `other_changes.entity` keys are `sheet:<id>`, `layout:<sheet>:<axis>:<idx>`, `chart:<id>` and `structure`.
- SQLite's automatic WAL checkpoint is raised to a large safety threshold, because a checkpoint during a commit costs ~200 ms. The engine checkpoints explicitly when idle.

Structural operations (insert or delete rows and columns) are stored as one `other_changes` record (`{"op":"insert_rows","sheet":1,"at":10,"n":3}`) plus `cell_changes` for any cells deleted. Their inverse is calculated, not stored.

### 7.3 Undo and redo semantics

- The **undo stack** is derived from `txns`. Undo applies the inverse of the latest normal or redo transaction that hasn't yet been undone, and records a `kind=undo` transaction that targets it. Redo is the mirror image. A new normal transaction clears the redo stack, but nothing is deleted.
- The stack is rebuilt from the database on open, so **undo survives restarts**.
- **Restore** (from per-cell history) writes a `kind=restore` transaction setting the cell back to a historical `(raw, meta)` pair.

### 7.4 Per-cell history

`SELECT … FROM cell_changes WHERE sheet_id=? AND row=? AND col=? ORDER BY txn DESC` uses the primary key directly. The panel joins `txns` for the timestamp and label, and `metatables` to show member-level diffs between consecutive versions.

Cell coordinates in history are **as of that transaction**. After a row insert, the panel follows the cell back through structural changes by mapping its coordinates backwards through each structural op.

---

## 8. User interface

### 8.1 Layout

```
┌────────────────────────────────────────────────────────────────────┐
│ budget.decel — Sheet1                                   ⌘ palette │  title bar (client-side decorations)
├───┬────────────────────────────────────────────────┬───────────────┤
│   │  1       2        3        4        5         │ ▾ Metatable   │
│ 1 │ Site     Lat      Lon      Elev               │  kind         │
│ 2 │ Alpha    35°18′S  149°7′E  577 m              │  unit         │
│ 3 │ Bravo    …                                    │ ▸ display     │  right dock (toggle):
│ … │                                               │   parse       │  Metatable | History
│   │                                               │ ┌───────────┐ │
│   │                                               │ │ code      │ │
│   │                                               │ └───────────┘ │
├───┴────────────────────────────────────────────────┴───────────────┤
│ 2,2 │ -35.307500                          Decimal degrees or D°M'S │  edit bar: address · edit text · hint
├────────────────────────────────────────────────────────────────────┤
│ Sheet1 │ Inputs │ + │                                 ⚠ 0  ⟳ idle │  sheet tabs · status
└────────────────────────────────────────────────────────────────────┘
```

- There is no ribbon and no toolbar. Every command is in the **command palette** (`Ctrl+Shift+P`) and in the keymap.
- The right dock is hidden by default and opens on `F4` (Metatable) or `F5` (History).
- Light and dark themes follow the OS by default.

### 8.2 Grid rendering

The grid is a **custom GPUI element**, not a widget table:

- **Virtualised.** Only visible cells are laid out and painted. Row heights and column widths live in a prefix-sum tree for O(log n) offset-to-index lookups. Sheets are sparse, so a million-row sheet costs only what's populated.
- Painting goes in this order: backgrounds, then metatable boundaries, then faint grid lines, then text (shaped runs cached by `(text, style)`), then markers (error and warning corners), then selection, then the fill handle, then overlays (peek and share highlight).
- Header row and column are frozen. User-frozen panes are post-v1.
- Text that overflows spills into empty neighbours to the right (left-aligned) or left (right-aligned). Otherwise it's clipped with an ellipsis.

### 8.3 Modes: normal, peek and code side

| Mode | Entered by | What it shows | Clicking a cell |
|---|---|---|---|
| **Normal** | default | Display output | Selects. Typing or `F2` edits the value. |
| **Peek** (quasimode) | **hold** `` ` `` (backtick, configurable) | Each cell is tinted by its metatable colour. A floating legend lists kind, unit and count per colour. Cells with no metatable are left untinted. | Same as normal |
| **Code side** (flip) | `F3` toggles, with a short flip animation | Tinted like peek. Each cell shows `kind` (or the first member name) instead of its value. Boundaries are emphasised. The Metatable dock opens. | Opens that cell's metatable (copy-on-write). With `Shift` held: hover shows the share set, and click opens a shared edit (§5.3). |

**Colour assignment.** Each distinct hash in the workbook gets a colour from a 16-hue OKLCH palette tuned separately for light and dark themes. Colours are assigned in order of first appearance (row-major, sheet order) and stored in workbook settings, so they stay stable between sessions. Beyond 16 hashes, colours repeat with a hatch pattern.

**Automatic boundaries.** A 1.5 px line is drawn between horizontally or vertically adjacent cells when their hashes differ and at least one of them is non-empty. It's neutral grey in normal mode and coloured in peek and the code side. It can be toggled with `Ctrl+Shift+B`.

### 8.4 The metatable editor (REALbasic style)

```
┌ Metatable ─────────────────────────────────┐
│ Editing this cell only · shared by 312     │  ← COW / shared banner
│ ┌─────────────┐ ┌────────────────────────┐ │
│ │ kind  "lat… │ │ function display(self, │ │  ← fixed signature (read-only)
│ │ unit  "deg" │ │          c, v)         │ │
│ │▸display     │ │   return fmt.dms(v,    │ │  ← body (tree-sitter Lua highlighting)
│ │ edit        │ │     "lat", 1)          │ │
│ │ parse       │ │ end                    │ │
│ │ validate  ⚠ │ │                        │ │
│ │ tooltip     │ ├────────────────────────┤ │
│ │ + add hook  │ │ ✓ compiles · preview:  │ │  ← live compile + preview against this cell
│ │ + helper    │ │ 35°18′27.0″S           │ │
│ └─────────────┘ └────────────────────────┘ │
│ log: (print output for this cell)          │
└────────────────────────────────────────────┘
```

- The member list shows a status per member: ✓ compiles, ⚠ runtime error on this cell, ✗ syntax error.
- "+ add hook" offers the standard hooks that aren't yet defined. "+ helper" adds a free-form member.
- The **preview** runs the edited (uncommitted) metatable against the selected cell in a scratch evaluation. It never affects the stored workbook.
- The code editor is gpui-component's code editor with the `tree-sitter-lua` feature.

### 8.5 Editing values

- Typing starts editing and replaces the content. `F2` or double-click edits with the text from `edit()` (or the default text for the raw value).
- The edit bar shows `edit_hint()`. `Enter` commits and moves down, `Tab` commits and moves right, and `Esc` cancels.
- If `parse` returns `(nil, err)` or `validate` fails, the editor stays open with the error message in red under the edit bar.
- Hovering a cell for 400 ms shows `tooltip()`, or the default tooltip of the value plus unit. Errors show their message and location.

### 8.6 Charts

- Select a range, run "Insert chart" from the palette, and a floating chart appears anchored to the top-left cell of the selection.
- The **Chart panel** (in the dock) sets the type (line, bar or scatter), title, the X range, Y series ranges (each with its own name and colour), axis titles, and a legend toggle.
- The spec is JSON in `charts.spec`. Changes are recorded in `other_changes`, so they can be undone.
- Charts are drawn with GPUI paths by a small built-in renderer that handles axes, nice ticks, lines, bars and points. There's no external plotting dependency. Charts update live as values recalculate.
- Charts move and resize with the mouse, and stay anchored to their cell when rows or columns are inserted.

### 8.7 Default keymap (all configurable)

| Action | Key |
|---|---|
| Command palette | `Ctrl+Shift+P` |
| Peek (hold) | `` ` `` |
| Code side toggle | `F3` |
| Share modifier (code side) | `Shift` |
| Metatable / History dock | `F4` / `F5` |
| Edit cell | `F2`, `Enter`, typing |
| Fill down / right | `Ctrl+D` / `Ctrl+R` |
| Auto-fill column / row | `Ctrl+Shift+D` / `Ctrl+Shift+R` |
| Copy / paste / paste values / paste meta | `Ctrl+C` / `Ctrl+V` / `Ctrl+Shift+V` / `Ctrl+Alt+V` |
| Undo / redo | `Ctrl+Z` / `Ctrl+Shift+Z`, `Ctrl+Y` |
| Toggle boundaries | `Ctrl+Shift+B` |
| Insert / delete row | `Ctrl+Shift+=` / `Ctrl+-` (with row selected) |
| New / open / save as | `Ctrl+N` / `Ctrl+O` / `Ctrl+Shift+S` |

The config lives at `$XDG_CONFIG_HOME/decel/config.toml` on Linux and `%APPDATA%\decel\config.toml` on Windows. It holds the keymap, theme, font, peek key, default column width, instruction budget and memory cap.

---

## 9. Architecture

### 9.1 Crates

```
decel/                        (workspace; all crates MIT OR Apache-2.0, published in lockstep)
├─ crates/decel-model         pure types: CellAddr, Value, MetaSource/MetaHash, Sheet (sparse), Ops, Txn
├─ crates/decel-store         SQLite schema, migrations, apply/invert txns, undo/redo, history queries, lock
├─ crates/decel-engine        Lua host, sandbox, stdlib (fmt/parse/color), compile cache,
│                             dependency graph + R-tree, recalculation, engine actor (thread + channels)
├─ crates/decel-io            CSV import/export
└─ crates/decel               the binary: GPUI app (grid element, dock, editor, history, charts, palette, config)
```

Dependency direction: `decel → decel-engine → decel-store → decel-model`, and `decel-io → decel-model`. Only the `decel` crate depends on GPUI. The engine is fully usable headless, which is what keeps a future web or CLI front end open.

### 9.2 Threads

```
UI thread (GPUI)                         Engine thread
──────────────────                       ──────────────────────────────────
GridView, Dock, Editor ...               owns: mlua::Lua, Model, Store (rusqlite), DepGraph
        │  Command (edit, fill, undo,          │
        │  set_meta, subscribe(viewport),      │
        │  preview_meta, query_history …)      │
        ├──────────── crossbeam channel ──────▶│  apply txn → SQLite commit → mark dirty
        │                                      │  recalc in ~5 ms slices, pre-emptible
        │◀──────────── Event channel ──────────┤  CellsUpdated{display cells for subscribed rect}
        │  (GPUI async task drains events,     │  TxnCommitted, RecalcProgress, Error, HistoryResult
        │   updates ViewportCache, notify())   │
```

- The UI keeps a **ViewportCache**: `DisplayCell { text, style, markers, hash, kind }` for the subscribed rectangle plus a margin. Scrolling only re-subscribes. The engine answers from cached display results or evaluates display hooks lazily.
- Every mutation goes through the engine as a command, so the engine is the single source of truth. The UI never touches SQLite or Lua.
- Commands carry a sequence number, and events acknowledge it. This lets the UI apply edits optimistically (show the typed text immediately) and reconcile when the event arrives.

Implementation notes (Phase 3, `decel_engine::protocol` and `actor`):

- Every command carries a `seq`. Each gets one `Ack { seq, result }`, whose `Reply` also carries query results (history, share set, metatable source, tooltip, edit text, preview, fill addresses). There are no separate result events.
- `Subscribe { id, sheet, rect }` supports several subscriptions (the grid, plus later the charts). Subscribing sends a full snapshot (`CellsUpdated { full: true }`) of the rectangle plus a 20-cell margin. After that, only changed cells inside a subscription are pushed. Visible dirty cells are evaluated when their display is computed, so the viewport is never waiting on an off-screen recalculation.
- Recalculation slices end after 5 ms, or as soon as a command is waiting. The WAL is checkpointed after 500 ms idle.
- `New { path: None }` creates an in-memory workbook that Save As turns into a file.
- Fill copies metatables only, keeping destination values, unless `with_values` is set (the value modifier). Fill down/right copies the first row or column of the selection over the rest. Auto-fill's run excludes the selected cell and goes in both directions, each direction using its own neighbour's hash.
- **Detach** gives the selected cells a private copy of their metatable by setting a `_copy` helper member to the lowest number that produces an unused hash. Identity is structural (D8), so a private copy has to differ in source.

### 9.3 Key dependencies

| Need | Crate | Notes |
|---|---|---|
| UI | `gpui-component =0.9.0` with feature `tree-sitter-lua`; `gpui-pre =0.3.11`; `gpui-pre-platform =0.3.11` | Use the exact `gpui-pre` that gpui-component pins (D3). |
| Lua | `mlua` 0.12 with `lua54`, `vendored`, `send` | `send` lets the engine own the VM on its own thread |
| SQLite | `rusqlite` 0.41 with `bundled` | |
| Hashing | `blake3` | |
| Spatial index | `rstar` | Range dependencies |
| Channels | `crossbeam-channel` | |
| CSV | `csv` | |
| Config | `serde`, `toml`, `directories` | |
| Errors / logs | `thiserror`, `anyhow` (binary only), `tracing` | |
| Tests | `proptest`, `insta`, `criterion` | |

---

## 10. Distribution and licensing

### 10.1 Install

```
cargo install decel
```

- All workspace crates are published to crates.io with the same version (`cargo publish --workspace`). **No git dependencies are allowed**, because crates.io rejects them. CI checks this with `cargo publish --workspace --dry-run`.
- Lua and SQLite are vendored, so the only system requirements are a C compiler and GPUI's platform libraries:
  - **Linux:** a C toolchain, `pkg-config`, and development packages for `libxkbcommon`, `wayland`, `libxcb`, `fontconfig`, `freetype` and `vulkan-loader`. The README lists exact package names for Arch/CachyOS, Debian/Ubuntu and Fedora.
  - **Windows:** the MSVC build tools (Visual Studio Build Tools, "Desktop development with C++").
- `cargo install --git <repo>` also works for unreleased builds.

### 10.2 GPUI version risk

GPUI moves quickly, and gpui-component pins a specific `gpui-pre` snapshot. decel pins `gpui-component` to an exact minor version and depends on exactly the `gpui-pre` version it requires. Upgrades are done deliberately, in their own commits. If a future gpui-component isn't published on crates.io in a usable form, the fallback is to depend on `gpui`/`gpui-pre` directly and build the small set of widgets decel needs (code editor input, list, tabs). That fallback is costed in the plan as a contingency.

### 10.3 Licence

- Every crate has `license = "MIT OR Apache-2.0"`. The repo root contains `LICENSE-MIT` and `LICENSE-APACHE`, and the README has the standard dual-licence contribution clause.
- Dependencies must be MIT, Apache-2.0, BSD, Zlib, ISC, Unicode or MPL-2.0 (file-level copyleft only). `cargo-deny` enforces this in CI. Lua (MIT) and SQLite (public domain) are fine.

---

## 11. Quality targets for v1.0

| Area | Target |
|---|---|
| Scrolling | 120 fps at 2560×1440 with 50 k visible-region cells populated (RX 7900 XTX); 60 fps on integrated graphics |
| Edit latency | Keypress to repaint under 8 ms; commit of a simple cell under 16 ms including the SQLite commit |
| Recalc | 1 edit with 1 k dependents under 16 ms; full recalc of 100 k computed cells under 1 s |
| Open | 100 k-cell workbook opened and displayed in under 1 s |
| History | Undo or redo of a 10 k-cell fill in under 100 ms; per-cell history query in under 10 ms with 1 M change rows |
| Robustness | No panic reachable from Lua code or file contents. A corrupt or locked file gives a clear message. Any Lua code can be interrupted. |
| Tests | The engine and store have ≥ 85 % line coverage. Property tests show that undo of everything restores the initial state, and that replaying history equals the materialised state. |

---

## 12. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| GPUI/gpui-component API churn or publishing gaps | High | High | Exact version pins. Phase 0 spike proves `cargo install` on both OSes. The UI layer is kept thin. Contingency in §10.2. |
| Runtime-recorded dependencies miss reads (e.g. via helpers) | Medium | High | All cell access goes through recording handles, and raw `rawget` access to engine internals is impossible because handles are userdata. A fuzz test compares incremental recalc with full recalc. |
| Long Lua code freezes the engine | Medium | Medium | Instruction hook, pre-emptible slices, memory cap |
| History growth | Medium | Low | `cell_changes` stores only before/after values. A "compact history before date" command is post-v1. |
| Code side and peek confuse new users | Low | Medium | On-screen legend, flip animation, first-run tips |
| Windows GPU or driver issues with GPUI | Medium | Medium | Test on the Legion laptop early (Phase 0) |

---

## 13. Post-v1 candidates

- Workbook **modules**: named Lua modules stored in the workbook and shown in the same REALbasic-style list, `require`-able from metatables
- History **timeline scrubber** and named checkpoints/branches
- XLSX import (values and basic formats)
- Persisted computed-value cache for faster opening of very large workbooks
- Lua language-server completions for the `c`/Range API
- Frozen panes, conditional "views" and filtering
- macOS support, then a web build (wasm with a different Lua backend and SQLite in OPFS)
- History compaction

---

## 14. Glossary

| Term | Meaning |
|---|---|
| **raw** | The value the user entered, after parsing. Stored. |
| **final value** | The output of `value()` if it's defined, otherwise raw. Used by other cells. |
| **metatable / meta** | The set of Lua members attached to a cell |
| **hash** | The content address of a metatable's canonical source (BLAKE3, 16 bytes) |
| **share set** | All cells whose metatable has the same hash |
| **COW edit** | An edit that creates a new metatable for the clicked cell only |
| **shared edit** | An edit applied to the whole share set (share modifier + click) |
| **peek** | Hold-key quasimode that tints cells by metatable |
| **code side** | Toggle mode that flips the grid to show and edit metatables |
| **txn** | One user action, recorded in history |
