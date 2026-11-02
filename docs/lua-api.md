# decel Lua API reference

Every decel cell can carry a **metatable**: a small set of Lua 5.4 members that decide how the
cell is computed, displayed, edited, parsed, validated and described. This document is the
complete reference for writing them. For using the application, see the
[user guide](user-guide.md).

## Contents

1. [Metatables and members](#1-metatables-and-members)
2. [Canonical form and hashing](#2-canonical-form-and-hashing)
3. [The value pipeline](#3-the-value-pipeline)
4. [The cell context `c`](#4-the-cell-context-c)
5. [Cell handles](#5-cell-handles)
6. [Range handles](#6-range-handles)
7. [Sheet handles](#7-sheet-handles)
8. [Display tables and style](#8-display-tables-and-style)
9. [Standard library: `fmt`, `parse`, `color`, `range`](#9-standard-library)
10. [Errors](#10-errors)
11. [The sandbox](#11-the-sandbox)
12. [Limits](#12-limits)
13. [Worked examples](#13-worked-examples)

Conventions used below: rows and columns are **1-based integers**, as in the UI. "Populated"
means a cell that has a raw value, a metatable, or both. "Final value" means the result of the
cell's `value` hook if it has one, otherwise its raw value.

---

## 1. Metatables and members

A metatable is an ordered set of **members**. Each member is edited on its own in the Metatable
dock. A member's kind is determined by its name:

| Kind | Names | What its source is |
|---|---|---|
| **Field** | `kind`, `unit`, `style` | A Lua expression, evaluated once when the metatable is compiled |
| **Hook** | `value`, `display`, `edit`, `parse`, `validate`, `tooltip`, `edit_hint` | The **body** of a function with a fixed signature |
| **Helper** | Any other valid Lua identifier that isn't a keyword | A Lua expression, evaluated once when the metatable is compiled. Usually `function(self, ...) ... end`. |

### 1.1 Fields

| Field | Type | Purpose |
|---|---|---|
| `kind` | string | A label for this kind of cell. Shown in the peek legend and on the code side, readable as `c.kind`/`h.kind`, and matched by `r:where_kind(...)`. |
| `unit` | string | A unit or currency tag (`"m"`, `"deg"`, `"AUD"`). Shown in the legend and the default tooltip; readable as `c.unit`/`h.unit`. It is a **tag only**: decel never converts or checks units. |
| `style` | table | Static style defaults: the keys of a [display table](#8-display-tables-and-style) except `text` |

```lua
-- kind
"latitude"
-- unit
"deg"
-- style
{ align = "right", mono = true }
```

### 1.2 Hooks

You write only the body. The editor shows the signature above it, read-only.

| Hook | Signature | Returns | Called |
|---|---|---|---|
| `value` | `value(self, c)` | the cell's final value: number, string, boolean or `nil` | During recalculation, whenever something it read has changed |
| `display` | `display(self, c, v)` | a string or a [display table](#8-display-tables-and-style) | Lazily, when the cell is visible. Not called when `v` is `nil`. |
| `edit` | `edit(self, c, v)` | the text put in the editor when editing starts | On `F2`/double-click. `v` is the **raw** value. Not called when the raw value is `nil`. |
| `parse` | `parse(self, c, text)` | the new raw value, or `nil, "message"` to reject | When the user commits typed or pasted text |
| `validate` | `validate(self, c, v)` | `true`, or `false, "message"` | On entry (rejects the edit) and after each recalculation of a computed cell (shows a warning) |
| `tooltip` | `tooltip(self, c, v)` | a string, or `nil` for no tooltip | On hover. Not called when `v` is `nil`. |
| `edit_hint` | `edit_hint(self, c)` | a string, or `nil` | While the cell is being edited |

Arguments:

- `self` is the metatable itself: a **read-only** table of all compiled members. Read fields as
  `self.kind` and call helpers as `self:helper(...)`. Writing to it raises
  `attempt to modify a read-only table`. Many cells share one `self`, so per-cell state must come
  from `c`, never from `self`.
- `c` is the [cell context](#4-the-cell-context-c).
- `v` is the cell's final value (for `display`, `tooltip` and computed-value `validate`) or the
  value being entered (for entry-time `validate`).
- `text` is exactly what the user typed.

Details worth knowing:

- **`parse` and empty text.** `parse` also receives empty text. Return `nil` with no message to
  clear the raw value, or `nil, "message"` to reject.
- **`validate` on computed values** receives the computed value even when it is `nil`, so guard
  for that if your `value` hook can return `nil` (`if v == nil then return true end`).
- A `validate` that returns `nil` (or `false` with no message) counts as a failure, with the
  message "invalid value". Return `true` explicitly to accept.
- **`edit` falls back** to the default edit text (the raw value as you'd type it) if it is
  missing or returns `nil`.
- `display` may return a number or boolean; it is converted with `tostring`.

### 1.3 Helpers

A helper is any member that isn't a standard field or hook. Its source is an expression,
evaluated once at compile time, and the result is stored in `self`:

```lua
-- haversine (a function helper)
function(self, lat1, lon1, lat2, lon2)
  local R, rad = 6371008.8, math.pi / 180
  local dlat, dlon = (lat2 - lat1) * rad, (lon2 - lon1) * rad
  local a = math.sin(dlat / 2) ^ 2
    + math.cos(lat1 * rad) * math.cos(lat2 * rad) * math.sin(dlon / 2) ^ 2
  return 2 * R * math.asin(math.sqrt(a))
end

-- gst_rate (a constant helper)
0.10
```

Call them from hooks as `self:haversine(a, b, c, d)` or read them as `self.gst_rate`.

Fields and helpers are evaluated **without** a cell, so they cannot read cells at compile time.
A helper function can read cells when it's *called*, as long as the hook passes `c` to it:
`self:first_point(c)`.

### 1.4 How members are compiled

Each member is compiled as its own chunk, named after the member. A hook body is wrapped as
`return function(<signature>) <body> end`; a field or helper as `return <expression>`. Errors
therefore report the member name and a line number **within that member's body**, starting at
line 1. A metatable compiles as a whole even if one member has an error: the other members work,
and only cells that use the broken member show the error.

---

## 2. Canonical form and hashing

A metatable is identified by its source. decel builds a canonical text from its members, sorted
by name:

```text
decel-meta/1
<name>\t<kind>\t<byte length>\n<source>\n
...
```

where `<kind>` is `field`, `hook` or `helper`. Before that, each member's source is
**normalised**: `\r\n` and `\r` become `\n`, trailing whitespace is removed from every line and
from the end, and exactly one trailing newline is added.

The metatable's **hash** is the first 16 bytes of the BLAKE3 hash of that text. Consequences:

- Two cells with identical metatable source always have the same hash, the same colour, and are
  in the same share set, even if they were typed separately.
- Member order doesn't matter (members are sorted by name).
- Trailing spaces and line-ending style don't matter.
- Everything else does: comments, indentation inside a line, helper names.
- A metatable's source never changes once stored. Editing a metatable creates a new hash and
  re-points cells to it (see the user guide on copy-on-write and shared edits).

Member names must be valid Lua identifiers (letters, digits, `_`, not starting with a digit) and
not Lua keywords. The standard hook names are always hooks.

---

## 3. The value pipeline

```text
typed text ─▶ parse(self, c, text)      (or the default parser)
          ─▶ validate(self, c, v)        reject → editor stays open with the message
          ─▶ store raw value
raw ──────▶ value(self, c)               (or raw, if there is no value hook)
          ─▶ final value                  used by other cells, charts and display
          ─▶ validate(self, c, v)        failure → warning marker + tooltip message
          ─▶ display(self, c, v)          (or the default format)
```

**The default parser**, used when there is no `parse` hook:

| Text | Result |
|---|---|
| empty | clears the raw value |
| `true` / `false` (any case) | boolean |
| `42`, `-7`, `1,000`, `1_000` | integer |
| `3.5`, `.5`, `1e3`, `1,234.5` | float |
| `'anything` | the string after the apostrophe |
| anything else | string |

**The default display** (no `display` hook, or `display` returned `nil`): integers as typed,
floats with up to 10 significant digits (scientific notation outside 1e-6 … 1e15), booleans as
`TRUE`/`FALSE`, strings as they are. Numbers are right-aligned and everything else left-aligned.

**Cell values** are `nil`, booleans, integers, floats and strings. Lua integers and floats stay
distinct: `return 2` stores an integer, `return 2.0` a float. A hook can't store a table,
function or handle as a value; `value` returning one gives `#TYPE`.

**Dates** are numbers: days since 1970-01-01 UTC, with any fraction as the time of day. Use
`fmt.date` and `parse.date` to show and read them.

---

## 4. The cell context `c`

Every hook receives `c`, a handle to the cell being evaluated. Reads made through `c` and the
handles it returns are **recorded as dependencies**, so the cell recalculates when what it read
changes. Reads that only happen on some paths are fine: dependencies are re-recorded on every
evaluation.

### 4.1 Fields

| Field | Value | Dependency? |
|---|---|---|
| `c.row`, `c.col` | this cell's row and column (1-based integers) | no |
| `c.sheet` | a [Sheet handle](#7-sheet-handles) for this cell's sheet | no |
| `c.raw` | this cell's own raw value | no (it's this cell's own input) |
| `c.kind`, `c.unit` | this cell's metatable `kind` and `unit`, or `nil` | no |

### 4.2 Single cells

Each returns a [Cell handle](#5-cell-handles). Creating a handle reads nothing; reading its
`.value` (or `.raw`, `.ok`…) does.

| Method | Refers to |
|---|---|
| `c:offset(dr, dc)` | The cell `dr` rows down and `dc` columns right. `c:offset(0, -1)` is the cell to the left. |
| `c:at(row, col)` | The cell at absolute `row, col` on this sheet |
| `c:up(n)`, `c:down(n)`, `c:left(n)`, `c:right(n)` | `n` cells in that direction; `n` defaults to 1 |

Handles may point off the sheet (row or column below 1). Such a handle has `.value == nil` and
`.raw == nil`, and its `.row`/`.col` report the off-sheet position (`c:up()` from row 1 has
`row == 0`). This makes "the cell above, if any" patterns simple:

```lua
local prev = c:up()
if prev.row >= 1 and prev.kind == self.kind then ... end
```

**Relative versus absolute.** `offset`, `up`, `down`, `left` and `right` are relative, so one
shared metatable works in every row, and they keep working when rows and columns are inserted or
deleted. `c:at(row, col)` is absolute: inserting a row does **not** rewrite the numbers in your
code, so `c:at(5, 2)` then reads whatever is now at row 5.

### 4.3 Ranges

Each returns a [Range handle](#6-range-handles).

| Method | Covers | Includes this cell? |
|---|---|---|
| `c:above()` | this column, rows `1 .. row-1` | no |
| `c:below()` | this column, rows `row+1 ..` (open-ended) | no |
| `c:left_of()` | this row, columns `1 .. col-1` | no |
| `c:right_of()` | this row, columns `col+1 ..` (open-ended) | no |
| `c:column(r1, r2)` | this column, rows `r1 .. r2`. `r1` defaults to 1; with no `r2` the range is open-ended. | **no, never** |
| `c:row_range(c1, c2)` | this row, columns `c1 .. c2`, defaults as for `column` | **no, never** |
| `c:rect(dr1, dc1, dr2, dc2)` | the rectangle between offsets `(dr1, dc1)` and `(dr2, dc2)` relative to this cell; the corners may be given in either order | yes, if the rectangle covers it |

**Excluding self.** The helpers that describe "the rest of this column or row" never include the
evaluating cell. A total at the bottom of a column can therefore use `c:column():sum()` without
creating a cycle, and so can a cell in the middle of the column.

**Open-ended ranges** (`c:below()`, `c:right_of()`, and `c:column()`/`c:row_range()` without an
end) automatically pick up cells added later, because the dependency is recorded on the whole
area, not on the cells that existed at the time.

`c:rect` parts that fall off the sheet are clipped. A rectangle entirely above or left of the
sheet is an empty range.

### 4.4 Other sheets

| Method | Returns |
|---|---|
| `c:sheet_named(name)` | A [Sheet handle](#7-sheet-handles) for the sheet called `name`. If no such sheet exists, it raises a `#REF` error. |

The lookup is recorded as a dependency on the **name**: renaming or deleting that sheet turns the
cell into `#REF`, and creating or renaming a sheet to that name fixes it again.

---

## 5. Cell handles

Returned by `c:offset`, `c:at`, `c:up` and friends, `h:offset`, `sheet:at`, `r:cells()`,
`r:map`/`r:filter` arguments, and `r:filter` results.

| Member | Value |
|---|---|
| `h.value` | That cell's **final value**. If the cell is dirty it is evaluated first. If the cell holds an error, reading `.value` **re-raises the error**, so it propagates into this cell. |
| `h.raw` | That cell's raw value |
| `h.ok` | `false` if the cell holds an error, otherwise `true`. Never raises. |
| `h.err` | The error message if the cell holds an error, otherwise `nil` |
| `h.kind`, `h.unit` | That cell's metatable `kind` and `unit`, or `nil` |
| `h.row`, `h.col` | Its position (1-based; may be 0 or negative for off-sheet handles) |
| `h:offset(dr, dc)` | A handle relative to this one |
| `tostring(h)` | `"cell(row,col)"` |

Every read except `row`/`col` records a dependency on that cell.

```lua
-- Use the cell to the left if it's healthy, else 0.
local left = c:left()
if not left.ok then return 0 end
return (left.value or 0) * 2
```

---

## 6. Range handles

A range is a rectangle on one sheet. A range method records a **range dependency** on the whole
rectangle (not one per cell), so `c:above():sum()` stays cheap with 100 000 rows above and also
reacts to cells added inside the rectangle later.

Every method looks only at **populated** cells inside the range, in row-major order (left to
right, then top to bottom), skipping the evaluating cell where the range excludes it.

| Method | Returns |
|---|---|
| `r:values()` | An array of the final values, with `nil`s skipped. If any cell holds an error, the error is **re-raised**. |
| `r:cells()` | An iterator of Cell handles, for `for h in r:cells() do ... end`. Includes populated cells whose value is `nil`. Reading values is up to you. |
| `r:sum()` | Sum of the numeric values (non-numbers are skipped). Stays an **integer** while every term is an integer (and doesn't overflow); otherwise a float. An empty range sums to `0`. |
| `r:min()`, `r:max()`, `r:avg()` | Minimum, maximum and mean of the numeric values, as floats. `nil` if there are no numbers. |
| `r:count()` | Number of non-`nil` values (numbers, strings and booleans all count) |
| `r:map(f)` | An array of `f(h)` for each populated cell, where `h` is a **Cell handle** |
| `r:filter(f)` | An array of the **Cell handles** `h` for which `f(h)` is true |
| `r:reduce(f, init)` | Folds over the **values** (with `nil`s skipped): `acc = f(acc, v)`, starting from `init` |
| `r:where_kind(kind)` | A new range covering the same area, limited to cells whose metatable `kind` equals `kind` |
| `r.rows`, `r.cols` | The range's size. For open-ended ranges, the size up to the last populated cell inside it (`0, 0` if there is none). |
| `tostring(r)` | `"range(r1,c1..r2,c2)"` |

Errors: `values`, `sum`, `min`, `max`, `avg`, `count` and `reduce` all read every value, so an
error anywhere in the range propagates. `map`, `filter` and `cells` hand you handles and only
raise if your function reads an error cell's `.value`. Check `h.ok` to skip error cells:

```lua
local total = 0
for h in c:above():cells() do
  if h.ok and type(h.value) == "number" then total = total + h.value end
end
return total
```

More examples:

```lua
c:above():sum()                                  -- running sum of the column so far
c:column():where_kind("money"):sum()             -- all money cells in this column but me
c:left_of():count()                              -- how many values to my left
c:rect(-2, 0, 2, 0):avg()                        -- 5-row moving average centred here
c:above():reduce(function(acc, v) return acc + v * v end, 0)    -- sum of squares
                                                  -- (assumes only numbers above)
#c:above():filter(function(h) return h.ok and h.value == "Rent" end)   -- count matches
```

---

## 7. Sheet handles

Returned by `c.sheet` and `c:sheet_named(name)`.

| Member | Value |
|---|---|
| `s.name` | The sheet's name (records a dependency on that name) |
| `s:at(row, col)` | A Cell handle at absolute `row, col` on that sheet |
| `s:range(r1, c1, r2, c2)` | A Range handle for that rectangle. Corners may be given in either order. If the smaller row or column is below 1 the range is empty. |
| `s:last_row()`, `s:last_col()` | The last row / column holding a populated cell, or 0 for an empty sheet |

`last_row()` and `last_col()` record a dependency on the **whole sheet**, so the cell recalculates
after any change there. That's correct but not cheap; prefer an open-ended range such as
`c:below()`, or a generous fixed bound such as `s:range(2, 1, 10000, 1)`, when you can.

```lua
local rates = c:sheet_named("Rates")
for h in rates:range(2, 1, 100, 1):cells() do
  if h.value == "USD" then return c:left().value * h:offset(0, 1).value end
end
error("no USD rate on the Rates sheet")
```

---

## 8. Display tables and style

`display` may return a plain string, or a **display table**:

```lua
{
  text   = "35°18′29″S",           -- the text to show (nil: the default text)
  fg     = "#c0392b",              -- text colour
  bg     = color.rgb(255, 240, 240), -- background colour
  align  = "right",                -- "left" | "centre" (or "center") | "right"
  bold   = false,
  italic = false,
  mono   = true,                   -- monospaced font
}
```

- Colours are `"#rrggbb"` or `"#rgb"` strings (with or without `#`, any case), such as the values
  returned by the `color` helpers.
- Every key is optional. Unknown keys and malformed values (a misspelt alignment, a non-boolean
  `bold`, an invalid colour) are ignored.
- The `style` field takes the same keys except `text`.

**Precedence**, for each key separately:

1. the table returned by `display`,
2. then the `style` field,
3. then the default: numbers right-aligned, everything else left-aligned, the normal proportional
   font, theme colours.

So `style = { align = "right", mono = true }` sets defaults for every cell of the kind, and
`display` can still override the colour of a single cell:

```lua
-- display
local t = { text = fmt.currency(v, "A$", 2) }
if v < 0 then t.fg = "#c0392b" end   -- red for negative; alignment comes from style
return t
```

---

## 9. Standard library

These modules are available inside every hook and helper, in addition to Lua's `string`, `math`,
`table` and `utf8` (see [the sandbox](#11-the-sandbox)).

General rules:

- **Formatters return strings.** If the value isn't a number, a `fmt` function returns
  `tostring(v)` instead of raising, so `fmt.fixed(nil)` is `"nil"` and `fmt.fixed("n/a")` is
  `"n/a"`. Non-finite numbers format as `NaN`, `∞` or `-∞` in every formatter.
- **Decimal-place and significant-figure arguments** are clamped to `0 … 15` (significant figures
  to at least 1).
- **Rounding** of `fixed`, `thousands`, `percent` and `currency` is exact on the binary value,
  with exact ties rounding half to even. `fmt.fixed(1.005, 2)` is `"1.00"` because the binary
  value of `1.005` is slightly below it. A result that rounds to zero never has a minus sign.
- **Parsers** trim surrounding whitespace, accept `−` (U+2212, the typographic minus) wherever
  `-` is accepted, return `nil` for unparseable text (or a `nil` argument), and always return a
  finite float.

### 9.1 `fmt`: formatting numbers for display

| Function | Default arguments | Example | Result |
|---|---|---|---|
| `fmt.fixed(v, dp)` | `dp = 2` | `fmt.fixed(1234.567, 2)` | `"1234.57"` |
| `fmt.thousands(v, dp)` | `dp = 0` | `fmt.thousands(1234567.891, 2)` | `"1,234,567.89"` |
| `fmt.percent(v, dp)` | `dp = 0` | `fmt.percent(0.125, 1)` | `"12.5%"` |
| `fmt.sci(v, sig)` | `sig = 3` | `fmt.sci(12345, 3)` | `"1.23e4"` |
| `fmt.si(v, unit, sig)` | `unit = ""`, `sig = 3` | `fmt.si(12500, "m", 3)` | `"12.5 km"` |
| `fmt.currency(v, symbol, dp)` | `symbol = "$"`, `dp = 2` | `fmt.currency(-1234.5, "$", 2)` | `"-$1,234.50"` |
| `fmt.dms(v, kind, sec_dp)` | `kind = nil`, `sec_dp = 0` | `fmt.dms(-35.3075, "lat", 0)` | `"35°18′27″S"` |
| `fmt.date(v, pattern)` | `pattern = "%Y-%m-%d"` | `fmt.date(19782, "%d %b %Y")` | `"29 Feb 2024"` |
| `fmt.duration(seconds)` | | `fmt.duration(90061)` | `"1d 1:01:01"` |

Details:

- **`fixed`**: `fmt.fixed(2.5, 0)` is `"2"`, `fmt.fixed(3.5, 0)` is `"4"` (half to even),
  `fmt.fixed(9.999, 2)` is `"10.00"`.
- **`thousands`**: `,` every three digits: `fmt.thousands(-1000)` is `"-1,000"`.
- **`percent`**: multiplies by 100: `fmt.percent(-0.5)` is `"-50%"`.
- **`sci`**: keeps trailing zeros: `fmt.sci(1e4, 3)` is `"1.00e4"`; zero is `"0"`.
- **`si`**: chooses an engineering prefix (a power of 1000) from `y` (10⁻²⁴) to `Y` (10²⁴),
  including `µ`, and trims trailing zeros: `fmt.si(0.0000015, "F")` is `"1.5 µF"`,
  `fmt.si(999.96, "V", 3)` is `"1 kV"`, `fmt.si(5, "m")` is `"5 m"`, `fmt.si(12500)` is
  `"12.5k"`. There is a space before the unit only when there is a unit. Outside the prefix range
  it falls back to `sci` plus the unit. This is display only; nothing is converted. Be careful
  with squared or cubed units: `fmt.si(0.005, "m²")` gives `"5 mm²"`, which is wrong (1 mm² is
  10⁻⁶ m²). Use `fmt.sci` for those.
- **`currency`**: `thousands` with the symbol placed after the sign:
  `fmt.currency(1e6, "AUD ", 0)` is `"AUD 1,000,000"`, `fmt.currency(12, "")` is `"12.00"`.
- **`dms`**: degrees, minutes and seconds with `sec_dp` decimals on the seconds. `kind` is `"lat"`
  (adds `N`/`S`), `"lon"` (adds `E`/`W`) or `nil` (signed, no letter); anything else is an
  error. Minutes and seconds aren't zero-padded. Rounding is half away from zero and carries 60″
  into minutes and degrees: `fmt.dms(149.1286, "lon", 2)` is `"149°7′42.96″E"`,
  `fmt.dms(-35.3075)` is `"-35°18′27″"`. Seconds decimals are capped at 9.
- **`date`**: `v` is days since 1970-01-01 UTC; a fraction is the time of day, rounded to the
  second. Pattern codes:

  | Code | Meaning | Example |
  |---|---|---|
  | `%Y` | year | `2024` |
  | `%y` | two-digit year | `24` |
  | `%m` | month, 2 digits | `02` |
  | `%b` / `%B` | month name, short / long | `Feb` / `February` |
  | `%d` | day of month, 2 digits | `29` |
  | `%a` / `%A` | weekday, short / long | `Thu` / `Thursday` |
  | `%j` | day of the year, 3 digits | `060` |
  | `%H` `%M` `%S` | hour, minute, second, 2 digits | `13:05:09` |
  | `%%` | a literal `%` | |

  Unknown codes are copied literally. Years before 1 are astronomical (`0000` is 1 BC). Values
  beyond ±10¹² days give `"####"`.
- **`duration`**: `[-][Nd ]H:MM:SS` from seconds, rounded to the second:
  `fmt.duration(3723)` is `"1:02:03"`, `fmt.duration(-5)` is `"-0:00:05"`.

### 9.2 `parse`: reading typed text

Use these in `parse` hooks. Each returns a float, or `nil` when the text doesn't parse.

| Function | Default arguments | Accepts | Example |
|---|---|---|---|
| `parse.number(text)` | | `1,234.5`, `1_000`, `-.5`, `+.5`, `1e3`, `5.` | `parse.number("1,234.5")` → `1234.5` |
| `parse.dms(text)` | | decimal degrees or D°M′S″ with an optional hemisphere | `parse.dms("35°18'27\"S")` → `-35.3075` |
| `parse.date(text, pattern)` | `pattern = "%Y-%m-%d"` | text matching `pattern` | `parse.date("29 Feb 2024", "%d %b %Y")` → `19782` |
| `parse.currency(text)` | | an amount with an optional symbol or code and sign | `parse.currency("(12.50)")` → `-12.5` |

Details:

- **`number`**: `,` and `_` are digit-group separators and must sit between digits of the integer
  part (group sizes aren't checked, so `"12,34"` is `1234`). Infinity, NaN, empty text and
  out-of-range values such as `"1e999"` are rejected. Note that `tonumber` also exists but
  doesn't accept separators.
- **`dms`**: components are numbers separated by spaces and/or the markers `°` (or `º`), `′` or
  `'` (minutes) and `″`, `"` or `''` (seconds). A marker fixes which component a number is;
  unmarked numbers fill degrees, minutes, seconds in order. Only the last component may have a
  fraction, minutes and seconds must be below 60, and a hemisphere letter `N`/`S`/`E`/`W` (any
  case) may come first or last; `S` and `W` make the result negative. A letter together with a
  minus sign is rejected. Degrees aren't range-checked, so validate that yourself. All of these
  give `-35.3075`: `"35 18 27 S"`, `"S35°18′27″"`, `"-35°18'27\""`, `"-35.3075"`.
- **`date`**: the inverse of `fmt.date`. Supports `%Y %m %d %H %M %S %y %b %B %a %A %%`;
  other pattern characters must match exactly. `%m %d %H %M %S` take one or two digits; `%y`
  maps `69`–`99` to the 1900s and the rest to the 2000s; month and weekday names are English, full
  or abbreviated, any case, and a weekday must agree with the date. Missing fields default to
  1970-01-01 00:00:00. Impossible dates (30 February, hour 24) are rejected. `%j` isn't supported
  for parsing.
- **`currency`**: at most one currency symbol or three-letter upper-case code, before or after the
  number (`$`, `€`, `£`, `¥`, `₹`, `A$`, `US$`, `AUD`, `kr` and others), and at most one sign: a
  leading `-`/`+` (before or after a leading symbol) or accounting parentheses. Spaces inside the
  amount are ignored, then the number rules apply. `"$1,234.50"`, `"-A$5"`, `"$-3"`,
  `"12.50 €"`, `"AUD 1,000"`, `"1 234.5"` all parse.

### 9.3 `color`: colours for display tables

Colours are lowercase `"#rrggbb"` strings. Functions that take colours accept anything
`color.hex` accepts.

| Function | Returns | Example |
|---|---|---|
| `color.rgb(r, g, b)` | `"#rrggbb"`; components are clamped to 0–255 and rounded | `color.rgb(255, 0, 128)` → `"#ff0080"` |
| `color.hex(s)` | `s` normalised to `"#rrggbb"`, or `nil` if it isn't a colour. Accepts `#abc`, `abc`, `#AABBCC`, `aabbcc`. | `color.hex("#ABC")` → `"#aabbcc"` |
| `color.lerp(a, b, t)` | the colour `t` of the way from `a` to `b` (per channel, `t` clamped to 0–1), or `nil` for an invalid colour | `color.lerp("#000", "#fff", 0.5)` → `"#808080"` |
| `color.scale(v, lo, hi, a, b)` | `color.lerp(a, b, (v - lo) / (hi - lo))`: `a` at `lo`, `b` at `hi`, clamped beyond. If `lo == hi` (or `v` is NaN) the result is `a`. `lo > hi` reverses the scale. | `color.scale(5, 0, 10, "#000", "#fff")` → `"#808080"` |

`color.scale` is the heat-map helper:

```lua
-- display: green at 0 %, red at 100 %
return { text = fmt.percent(v, 1), bg = color.scale(v, 0, 1, "#d5f5e3", "#f5b7b1") }
```

All arguments of `color.rgb`, `color.lerp` and `color.scale` are required and must be numbers or
strings as shown; passing `nil` raises an error.

### 9.4 `range`: variadic aggregates

| Function | Returns |
|---|---|
| `range.sum(...)` | Sum of all numbers given, as for `r:sum()` (integer while every term is) |
| `range.min(...)`, `range.max(...)` | Minimum / maximum as a float, or `nil` if there are no numbers |

Each argument may be a number, a Range handle (its values are included, with non-numbers
skipped and errors re-raised) or `nil` (ignored). Strings and other types raise an error.

```lua
return range.sum(c:rect(-3, -1, -1, -1), c:left().value, 100)
return range.max(c:sheet_named("Q1"):range(2, 3, 50, 3), c:sheet_named("Q2"):range(2, 3, 50, 3))
```

---

## 10. Errors

### 10.1 Error kinds

When a `value` hook fails, the cell's value becomes an error, shown as a code:

| Code | Raised when |
|---|---|
| `#LUA` | Any Lua runtime error: a bug, `error("...")`, `assert(false, "...")`, a syntax error in the member, a failed standard-library call. The message records the member and line. |
| `#CYCLE` | The cell depends on itself. Every cell in the cycle gets `#CYCLE`. |
| `#TIMEOUT` | The evaluation exceeded its instruction budget (or the half-second wall-clock safety net) |
| `#MEMORY` | The Lua VM ran out of its memory allowance, or a `string.rep`/concatenation result would be too large |
| `#REF` | `c:sheet_named(name)` found no sheet with that name |
| `#TYPE` | `value` returned something that can't be a cell value (a table, function or handle) |
| `#INVALID` | Reserved for validation errors. Validation currently never produces an error value: entry-time failures reject the edit, and computed-value failures show a warning. |

Hover over an error cell to see its message, member and line.

### 10.2 Propagation

Reading `h.value` of an error cell **re-raises that error** in the reading cell, keeping its kind:
a cell that reads a `#REF` cell shows `#REF`, not `#LUA`. Range methods that read values do the
same. A cell that reads a member of a cycle gets the `#CYCLE` error propagated, but isn't itself
part of the cycle.

To handle errors instead of propagating them, check first:

```lua
local h = c:left()
if not h.ok then return "n/a: " .. h.err end
return h.value * 2
```

`pcall` also catches propagated errors and ordinary Lua errors:

```lua
local ok, v = pcall(function() return c:left().value * 2 end)
if not ok then return 0 end
return v
```

### 10.3 Errors in other hooks

| Hook | On error |
|---|---|
| `display` | The default display is shown, with a red corner marker. The grid never breaks. |
| `tooltip` | The tooltip says `tooltip failed: …` |
| `edit` | The default edit text is used |
| `edit_hint` | No hint |
| `parse` | The edit is rejected with `parse failed: …` |
| `validate` | On entry, the edit is rejected with `validate failed: …`. After recalculation, a warning with that message. |

---

## 11. The sandbox

Metatables run in a **pure sandbox**, so it's safe to open workbooks written by other people. No
metatable can touch files, the network, the clock or the operating system, or leave state behind
for another cell.

### 11.1 What's available

| Available | |
|---|---|
| Base functions | `assert`, `error`, `ipairs`, `next`, `pairs`, `select`, `tonumber`, `tostring`, `type`, `rawequal`, `rawget`, `rawlen`, `getmetatable`, `pcall`, `xpcall`, `print`, `_VERSION`, `_G` |
| `string` | everything except `string.dump`. String methods work: `text:gsub(...)`, `s:upper()`. |
| `math` | everything except `math.random` and `math.randomseed` |
| `table`, `utf8` | everything |
| decel | `fmt`, `parse`, `color`, `range` |

### 11.2 What's removed

`io`, `os`, `debug`, `coroutine`, `package`, `require`, `load`, `loadstring`, `loadfile`,
`dofile`, `collectgarbage`, `setmetatable`, `rawset`, `string.dump`, `math.random`,
`math.randomseed`. There is no way to read the time.

### 11.3 Read-only globals

The global environment is frozen. Assigning a global raises
`globals are read-only (use local)`. Always use `local`:

```lua
x = 1          -- error: globals are read-only (use local)
local x = 1    -- fine
```

`string`, `math`, `table`, `utf8` and the decel modules are read-only too, and so is `self`.
`getmetatable` works, but reports `false` for strings and `"locked"` for every frozen table.
Creating and modifying your own local tables is fine (`local t = {}; t.x = 1`).

### 11.4 `print`

`print(...)` writes to the per-cell log shown at the bottom of the Metatable dock, joined by tabs,
up to 200 lines per evaluation. It's the quickest way to debug.

### 11.5 `pcall` can't catch interrupts

`pcall` and `xpcall` are thin wrappers around the real ones. They catch ordinary errors with their
original error values, but **re-raise** timeouts and memory-limit errors (and the engine's
internal "evaluate this other cell first" signal). A script therefore can't catch its own timeout
and keep looping; the cell shows `#TIMEOUT` regardless.

### 11.6 Determinism

With no random numbers, no clock and no shared mutable state, a metatable's result depends only on
the cells it reads. Recalculating gives the same answer every time, which is what keeps history
and undo meaningful. If you need pseudo-random data, derive it from the cell's position
(`c.row`, `c.col`).

---

## 12. Limits

| Limit | Default | Setting | When exceeded |
|---|---|---|---|
| Instructions per evaluation | 10 000 000 VM instructions | `instruction_budget` in `config.toml` | `#TIMEOUT` |
| Wall-clock time per evaluation | 500 ms (a safety net) | — | `#TIMEOUT` |
| Lua memory (the whole VM) | 512 MiB | `memory_limit_mb` in `config.toml` | `#MEMORY` |
| `print` lines per evaluation | 200 | — | further lines are dropped |

"Per evaluation" means per hook call. A hook that reads a cell which hasn't been calculated yet is
paused while that cell is calculated, then run again, so long dependency chains (such as a
500-row running total) never hit Lua's recursion limits.

---

## 13. Worked examples

Each example lists the members to create. "Body" is what goes in the editor for that member.
The [example workbooks](../examples/README.md) contain working versions of most of them.

### 13.1 Latitude: displayed as D°M′S″, edited as decimal

| Member | Body |
|---|---|
| `kind` | `"latitude"` |
| `unit` | `"deg"` |
| `display` | `return fmt.dms(v, "lat", 1)` |
| `edit` | `return string.format("%.6f", v)` |
| `parse` | see below |
| `validate` | see below |
| `tooltip` | `return string.format("%.6f° — %s", v, fmt.dms(v, "lat", 2))` |
| `edit_hint` | `return "Decimal degrees (−35.3075) or 35°18'27\"S"` |

```lua
-- parse
local n = tonumber(text) or parse.dms(text)
if not n then return nil, "Enter decimal degrees or D°M'S\" N/S" end
return n
```

```lua
-- validate
if type(v) ~= "number" or v < -90 or v > 90 then
  return false, "Latitude must be within ±90°"
end
return true
```

The cell stores `-35.3075`, shows `35°18′27.0″S`, opens in the editor as `-35.307500`, and accepts
`-35.3075`, `35 18 27 S` or `35°18'27"S`. Typing `95` is rejected with the message.

### 13.2 Running total, shared by filling down

| Member | Body |
|---|---|
| `kind` | `"running total"` |
| `value` | see below |
| `style` | `{ align = "right", mono = true }` |

```lua
-- value
local prev = c:up()
local base = (prev.row >= 1 and prev.kind == self.kind) and prev.value or 0
return base + c:left().value
```

Put amounts in a column, give the cell to their right this metatable, and fill it down (`Ctrl+D`)
or auto-fill (`Ctrl+Shift+D`). Every cell shares one metatable and one colour, and each still
computes its own total because the code only uses relative references. The first total has no
running-total cell above it, so it starts from 0.

### 13.3 Currency (Australian dollars)

| Member | Body |
|---|---|
| `kind` | `"money"` |
| `unit` | `"AUD"` |
| `display` | see below |
| `edit` | `return fmt.fixed(v, 2)` |
| `parse` | see below |
| `validate` | `if type(v) ~= "number" then return false, "Amounts must be numbers" end`<br>`return true` |
| `edit_hint` | `return "Amount in AUD; negative for spending"` |
| `style` | `{ mono = true }` |

```lua
-- display
local t = { text = fmt.currency(v, "A$", 2) }
if v < 0 then t.fg = "#c0392b" end
return t
```

```lua
-- parse
local n = parse.currency(text)
if not n then return nil, "Enter an amount like -1,234.50 or A$1,234.50" end
return n
```

`-2150` shows as `-A$2,150.00` in red. You can type `2150`, `-A$2,150.00`, `(2,150)` or
`AUD -2150`. A total row can then use `c:above():where_kind("money"):sum()`.

### 13.4 Percentage of total with a colour scale

This cell shows its row's amount (two columns to the left) as a share of all amounts with the same
sign, shaded from white to orange.

| Member | Body |
|---|---|
| `kind` | `"share"` |
| `amounts` (helper) | see below |
| `value` | see below |
| `display` | see below |

```lua
-- amounts (helper): the money cells in the amount column
function(self, c)
  return c.sheet:range(2, c.col - 2, 10000, c.col - 2):where_kind("money")
end
```

```lua
-- value
local amt = c:left(2).value
if type(amt) ~= "number" or amt == 0 then return nil end
local total = self:amounts(c):reduce(function(acc, v)
  if (v < 0) == (amt < 0) then return acc + v end
  return acc
end, 0)
return amt / total
```

```lua
-- display
return {
  text = fmt.percent(v, 1),
  bg = color.scale(v, 0, 0.5, "#ffffff", "#f5b041"),
  align = "right",
}
```

`where_kind("money")` keeps the header and any total row out of the sum, and `reduce` receives
plain values. Shares of 50 % and above get the full orange.

### 13.5 Unit-tagged engineering values with `fmt.si`

An input metatable that tags values with a unit, shows them with an engineering prefix, and lets
the user type the unit:

| Member | Body |
|---|---|
| `kind` | `"load"` |
| `unit` | `"N"` |
| `display` | `return fmt.si(v, self.unit, 4)` |
| `parse` | see below |
| `edit_hint` | `return "Number in " .. self.unit` |

```lua
-- parse
local s = text:gsub("%s*" .. self.unit .. "$", "")
local n = parse.number(s)
if not n then return nil, "Enter a number in " .. self.unit end
return n
```

`500000` shows as `500 kN`; typing `480000 N` stores `480000`. With the same body and
`unit = "m"`, `0.05` shows as `50 mm`.

A sum that warns when its terms have mixed units:

| Member | Body |
|---|---|
| `kind` | `"load sum"` |
| `unit` | `"N"` |
| `terms` (helper) | see below |
| `value` | see below |
| `validate` | see below |
| `display` | `return { text = fmt.si(v, self.unit, 4), bold = true }` |

```lua
-- terms (helper): handles for the unbroken run of numbers directly above
function(self, c)
  local out, n = {}, 1
  while true do
    local h = c:up(n)
    if h.row < 1 or type(h.value) ~= "number" then return out end
    out[#out + 1] = h
    n = n + 1
  end
end
```

```lua
-- value
local total = 0
for _, h in ipairs(self:terms(c)) do total = total + h.value end
return total
```

```lua
-- validate
local seen, units = {}, {}
for _, h in ipairs(self:terms(c)) do
  local u = h.unit or "(none)"
  if not seen[u] then seen[u] = true; units[#units + 1] = u end
end
table.sort(units)
if #units > 1 then return false, "Mixed units: " .. table.concat(units, ", ") end
if units[1] and units[1] ~= self.unit then
  return false, "Expected " .. self.unit .. " but the terms are in " .. units[1]
end
return true
```

If one of the terms is tagged `m` by mistake, the sum still computes (units are tags only), but
it gets a warning marker and the tooltip says `⚠ Mixed units: N, m`.

### 13.6 A date column

| Member | Body |
|---|---|
| `kind` | `"date"` |
| `display` | `return fmt.date(v, "%d %b %Y")` |
| `edit` | `return fmt.date(v, "%Y-%m-%d")` |
| `parse` | see below |
| `tooltip` | `return fmt.date(v, "%A %d %B %Y")` |
| `edit_hint` | `return "Date: 2026-03-14 or 14 Mar 2026"` |
| `style` | `{ align = "right" }` |

```lua
-- parse
local d = parse.date(text, "%Y-%m-%d") or parse.date(text, "%d %b %Y")
if not d then return nil, "Enter a date as 2026-03-14 or 14 Mar 2026" end
return d
```

The cell stores a day number (`20526` for 14 March 2026), shows `14 Mar 2026`, edits as
`2026-03-14` and has the weekday in its tooltip. Because dates are numbers, `r:min()`,
`r:max()` and subtraction work on them directly: the number of days between two dates is
`c:left().value - c:left(2).value`.

### 13.7 A cross-sheet lookup with `c:sheet_named`

A sheet called **Rates** holds currency codes in column 1 and rates per AUD in column 2. On the
budget sheet, the header of each conversion column names the currency (`USD`, `EUR`), and the
amount is in column 3. One metatable serves every conversion column:

| Member | Body |
|---|---|
| `kind` | `"converted"` |
| `rate` (helper) | see below |
| `value` | see below |
| `display` | `return fmt.thousands(v, 2) .. " " .. c:at(1, c.col).value` |
| `style` | `{ align = "right", mono = true, italic = true }` |

```lua
-- rate (helper)
function(self, c, code)
  local rates = c:sheet_named("Rates")
  for h in rates:range(2, 1, 100, 1):cells() do
    if h.value == code then return h:offset(0, 1).value end
  end
  error("no rate for " .. tostring(code) .. " on the Rates sheet")
end
```

```lua
-- value
local amt = c:at(c.row, 3).value
if type(amt) ~= "number" then return nil end
return amt * self:rate(c, c:at(1, c.col).value)
```

Changing a rate on the Rates sheet updates every conversion. Renaming the Rates sheet turns them
all into `#REF`; renaming it back fixes them. A currency missing from the table gives `#LUA` with
the message from `error(...)`. Note that `c:at(1, c.col)` and `c:at(c.row, 3)` are absolute
references: if you insert a column before column 3, update the code.

### 13.8 Distance from the first point (haversine helper)

On a sheet with latitudes three columns to the left and longitudes two to the left:

| Member | Body |
|---|---|
| `kind` | `"distance"` |
| `unit` | `"m"` |
| `haversine` (helper) | the function from [section 1.3](#13-helpers) |
| `first` (helper) | see below |
| `value` | see below |
| `display` | `return fmt.si(v, self.unit, 3)` |

```lua
-- first (helper): the first latitude cell in the latitude column
function(self, c)
  local lats = c.sheet:range(1, c.col - 3, c.row, c.col - 3):where_kind("latitude")
  for h in lats:cells() do return h end
  return nil
end
```

```lua
-- value
local first = self:first(c)
local lat, lon = c:left(3).value, c:left(2).value
if not first or type(lat) ~= "number" or type(lon) ~= "number" then return nil end
return self:haversine(first.value, first:offset(0, 1).value, lat, lon)
```

The first point shows `0 m` and the others `4.4 km`, `5.21 km` and so on.
