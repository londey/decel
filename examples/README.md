# Example workbooks

Three small workbooks that show what decel's Lua metatables can do. They aren't stored in the
repository (`.decel` files are binary SQLite databases). Generate them with the development
CLI:

```sh
cargo run -p decel-cli -- examples examples/
```

This writes `survey.decel`, `budget.decel` and `engineering.decel` into `examples/` (any directory
works, and it's created if needed). The command refuses to overwrite existing files, so delete
the old ones first to regenerate them. Please don't commit the generated files. Then open one:

```sh
cargo run --release -p decel -- examples/budget.decel
```

Hold `` ` `` to see the cell kinds, press `F3` to flip to the code side, and click a cell to read
its metatable in the dock. You can also check them headlessly; this prints a sheet's displayed
text as CSV:

```sh
cargo run -p decel-cli -- eval examples/budget.decel --sheet Rates
```

## `survey.decel`

Sheet **Survey**: six survey points around Canberra.

| Column | Metatable | Shows |
|---|---|---|
| Latitude, Longitude | `latitude` / `longitude` (DESIGN §4.7) | Stored as decimal degrees, shown as `35°18′29.2″S`, edited as decimal, typed either way, range-checked |
| Elevation | `elevation`, `unit = "m"` | `fmt.si` display (`812 m`); typing `812 m` or `812` both work |
| From first | `distance` | Great-circle distance from the first point, computed by a `haversine` helper member; finds the first point with `where_kind("latitude")` |
| Surveyed | `date` | Day numbers shown as `14 Mar 2026`, edited as `2026-03-14`, weekday in the tooltip |
| Highest (last row) | `column max` | `c:above():max()` |

## `budget.decel`

Sheet **Budget**: a month of income and spending in Australian dollars. Sheet **Rates**: exchange
rates per AUD.

| Column | Metatable | Shows |
|---|---|---|
| Amount | `money`, `unit = "AUD"` | `fmt.currency` (red when negative), `parse.currency` (`-A$2,150`, `(2150)`, `AUD 2150`) |
| Balance | `balance` | A running balance (DESIGN §4.8) |
| Share | `share` | Each line as a percentage of all income or all spending, with a `color.scale` background |
| USD, EUR | `converted` | One shared metatable: reads the currency code from its column header and looks the rate up on the Rates sheet with `c:sheet_named("Rates")` |
| Total (last row) | `total` | `c:above():where_kind("money"):sum()` |
| Category totals (columns 9–10) | `category total` | `r:filter` over the Category column |

Try changing a rate on the Rates sheet, or renaming the sheet (the conversions turn into `#REF`
until you rename it back).

## `engineering.decel`

Sheet **Beam**: a simple stress check on a rectangular section.

| Rows | Metatable | Shows |
|---|---|---|
| Dead load, Live load | `load`, `unit = "N"` | Unit-tagged inputs with `fmt.si` display (`500 kN`) |
| Force | `load sum` | Sum of the loads above, with a `validate` hook that warns on mixed units |
| Width, Depth | `length`, `unit = "m"` | `50 mm`, `100 mm` |
| Area, Stress | `area`, `stress` | Computed from rows found by label; `validate` checks the units of the inputs |
| Yield strength | `strength`, `unit = "Pa"` | `250 MPa` |
| Utilisation | `utilisation` | Percentage with a green-to-red `color.scale`; warns above 100 % |
| Factor of safety | `factor of safety` | Warns below 1.5 |
| Mixed-unit check | `load sum` | A deliberate mistake: one term is tagged `m`, so the sum shows a warning marker. Hover it to read `⚠ Mixed units: N, m`. |

Try raising the dead load to `1200000 N`: the utilisation turns red and goes over 100 %, and both it and the factor of safety get warning markers.

See the [user guide](../docs/user-guide.md) and the [Lua API reference](../docs/lua-api.md) for
how all of this works.
