// SPDX-License-Identifier: MIT OR Apache-2.0

//! Example workbooks for the user guide (`decel-cli examples <dir>`).
//!
//! - `survey.decel`: survey points with latitude/longitude metatables,
//!   elevations tagged `"m"`, survey dates and a computed distance from the
//!   first point (a haversine helper member).
//! - `budget.decel`: AUD budget lines with a currency metatable, a running
//!   balance, a percentage-of-total column with a colour scale, category
//!   totals, and conversions that look rates up on a second sheet ("Rates")
//!   with `c:sheet_named`.
//! - `engineering.decel`: a beam stress check with unit-tagged inputs,
//!   computed area/stress/utilisation, `fmt.si` display, and a `validate`
//!   hook that warns when summed values have mixed units.

use std::path::Path;

use anyhow::{Result, bail};
use decel_model::{Cell, CellAddr, MetaSource, Op, SheetId, TxnInput, Value};
use decel_store::Store;

use crate::demo;

fn s(v: &str) -> Cell {
    Cell::value(Value::str(v))
}

fn num(v: f64) -> Cell {
    Cell::value(Value::Num(v))
}

fn int(v: i64) -> Cell {
    Cell::value(Value::Int(v))
}

/// Creates a new workbook file, refusing to overwrite an existing one.
fn create(dir: &Path, name: &str) -> Result<Store> {
    let path = dir.join(name);
    if path.exists() {
        bail!("{} already exists; delete it first", path.display());
    }
    Ok(Store::create(&path)?)
}

/// Writes all three example workbooks into `dir` (created if missing).
pub fn write_all(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    survey(dir)?;
    budget(dir)?;
    engineering(dir)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// survey.decel

/// A numeric input tagged with a unit, shown with an engineering prefix.
/// Typed text may end with the unit (`"577 m"`).
pub fn unit_input(kind: &str, unit: &str) -> MetaSource {
    MetaSource::default()
        .with("kind", &format!("{kind:?}"))
        .with("unit", &format!("{unit:?}"))
        .with("display", "return fmt.si(v, self.unit, 4)")
        .with(
            "parse",
            r##"local s = text:gsub("%s*" .. self.unit .. "$", "")
local n = parse.number(s)
if not n then return nil, "Enter a number in " .. self.unit end
return n"##,
        )
        .with(
            "validate",
            r##"if type(v) ~= "number" then return false, "Enter a number" end
return true"##,
        )
        .with("edit_hint", r##"return "Number in " .. self.unit"##)
}

pub fn survey_date() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"date\"")
        .with("display", r##"return fmt.date(v, "%d %b %Y")"##)
        .with("edit", r##"return fmt.date(v, "%Y-%m-%d")"##)
        .with(
            "parse",
            r##"local d = parse.date(text, "%Y-%m-%d") or parse.date(text, "%d %b %Y")
if not d then return nil, "Enter a date as 2026-03-14 or 14 Mar 2026" end
return d"##,
        )
        .with("tooltip", r##"return fmt.date(v, "%A %d %B %Y")"##)
        .with("edit_hint", r##"return "Date: 2026-03-14 or 14 Mar 2026""##)
        .with("style", "{ align = \"right\" }")
}

/// Great-circle distance from the first latitude in the sheet, three
/// columns to the left (latitude) and two to the left (longitude).
pub fn distance() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"distance\"")
        .with("unit", "\"m\"")
        .with(
            "haversine",
            r##"function(self, lat1, lon1, lat2, lon2)
  local R = 6371008.8 -- mean Earth radius in metres
  local rad = math.pi / 180
  local dlat, dlon = (lat2 - lat1) * rad, (lon2 - lon1) * rad
  local a = math.sin(dlat / 2) ^ 2
    + math.cos(lat1 * rad) * math.cos(lat2 * rad) * math.sin(dlon / 2) ^ 2
  return 2 * R * math.asin(math.sqrt(a))
end"##,
        )
        .with(
            "first",
            r##"function(self, c)
  -- The first latitude cell in the latitude column, from the top.
  local lats = c.sheet:range(1, c.col - 3, c.row, c.col - 3):where_kind("latitude")
  for h in lats:cells() do return h end
  return nil
end"##,
        )
        .with(
            "value",
            r##"local first = self:first(c)
local lat, lon = c:left(3).value, c:left(2).value
if not first or type(lat) ~= "number" or type(lon) ~= "number" then return nil end
return self:haversine(first.value, first:offset(0, 1).value, lat, lon)"##,
        )
        .with("display", "return fmt.si(v, self.unit, 3)")
        .with(
            "tooltip",
            r##"local first = self:first(c)
return string.format("%.1f m from %s", v, tostring(first:offset(0, -1).value))"##,
        )
        .with("style", "{ align = \"right\", mono = true }")
}

pub fn column_max() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"column max\"")
        .with("unit", "\"m\"")
        .with("value", "return c:above():max()")
        .with(
            "display",
            "return { text = fmt.si(v, self.unit, 4), bold = true }",
        )
}

fn survey(dir: &Path) -> Result<()> {
    let mut store = create(dir, "survey.decel")?;
    let sh = SheetId(1);
    let a = |r: u32, c: u32| CellAddr::new(sh, r, c);
    let (lat, lon) = (demo::latitude(), demo::longitude());
    let elev = unit_input("elevation", "m");
    let (date, dist, max) = (survey_date(), distance(), column_max());
    let mut cells = vec![
        (a(0, 0), s("Point")),
        (a(0, 1), s("Latitude")),
        (a(0, 2), s("Longitude")),
        (a(0, 3), s("Elevation")),
        (a(0, 4), s("From first")),
        (a(0, 5), s("Surveyed")),
    ];
    let points = [
        ("Capital Hill", -35.3081, 149.1245, 603.0, 20526),
        ("Black Mountain", -35.2753, 149.0973, 812.0, 20527),
        ("Mount Ainslie", -35.2700, 149.1580, 843.0, 20533),
        ("Red Hill", -35.3360, 149.1220, 722.5, 20545),
        ("Mount Taylor", -35.3700, 149.0640, 855.4, 20552),
        ("Mount Majura", -35.2420, 149.1880, 888.0, 20561),
    ];
    for (i, (name, la, lo, el, day)) in points.iter().enumerate() {
        let r = i as u32 + 1;
        cells.push((a(r, 0), s(name)));
        cells.push((a(r, 1), num(*la).with_meta(lat.hash())));
        cells.push((a(r, 2), num(*lo).with_meta(lon.hash())));
        cells.push((a(r, 3), num(*el).with_meta(elev.hash())));
        cells.push((a(r, 4), Cell::EMPTY.with_meta(dist.hash())));
        cells.push((a(r, 5), int(*day).with_meta(date.hash())));
    }
    let last = points.len() as u32 + 1;
    cells.push((a(last, 0), s("Highest")));
    cells.push((a(last, 3), Cell::EMPTY.with_meta(max.hash())));
    store.apply(
        TxnInput::new("Survey example")
            .op(Op::RenameSheet {
                id: sh,
                name: "Survey".into(),
            })
            .op(Op::SetCells(cells))
            .meta(lat)
            .meta(lon)
            .meta(elev)
            .meta(date)
            .meta(dist)
            .meta(max),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// budget.decel

/// Australian dollars: shown as `A$1,234.50` (red when negative), typed
/// with or without the symbol.
pub fn aud() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"money\"")
        .with("unit", "\"AUD\"")
        .with(
            "display",
            r##"local t = { text = fmt.currency(v, "A$", 2) }
if v < 0 then t.fg = "#c0392b" end
return t"##,
        )
        .with("edit", "return fmt.fixed(v, 2)")
        .with(
            "parse",
            r##"local n = parse.currency(text)
if not n then return nil, "Enter an amount like -1,234.50 or A$1,234.50" end
return n"##,
        )
        .with(
            "validate",
            r##"if type(v) ~= "number" then return false, "Amounts must be numbers" end
return true"##,
        )
        .with(
            "edit_hint",
            r##"return "Amount in AUD; negative for spending""##,
        )
        .with("style", "{ mono = true }")
}

/// Running balance (DESIGN §4.8, with a currency display).
pub fn balance() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"balance\"")
        .with("unit", "\"AUD\"")
        .with(
            "value",
            r##"local prev = c:up()
local base = (prev.row >= 1 and prev.kind == self.kind) and prev.value or 0
return base + (c:left().value or 0)"##,
        )
        .with(
            "display",
            r##"local t = { text = fmt.currency(v, "A$", 2) }
if v < 0 then t.fg = "#c0392b" end
return t"##,
        )
        .with("style", "{ align = \"right\", mono = true }")
}

/// This line's amount as a share of all amounts of the same sign.
pub fn share() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"share\"")
        .with(
            "amounts",
            r##"function(self, c)
  -- The money cells in the amount column (two to the left).
  return c.sheet:range(2, c.col - 2, 10000, c.col - 2):where_kind("money")
end"##,
        )
        .with(
            "value",
            r##"local amt = c:left(2).value
if type(amt) ~= "number" or amt == 0 then return nil end
local total = self:amounts(c):reduce(function(acc, v)
  if (v < 0) == (amt < 0) then return acc + v end
  return acc
end, 0)
return amt / total"##,
        )
        .with(
            "display",
            r##"return {
  text = fmt.percent(v, 1),
  bg = color.scale(v, 0, 0.5, "#ffffff", "#f5b041"),
  align = "right",
}"##,
        )
        .with(
            "tooltip",
            r##"local kind = c:left(2).value < 0 and "spending" or "income"
return fmt.percent(v, 2) .. " of all " .. kind"##,
        )
}

/// Converts the amount in column 3 into the currency named in this
/// column's header, using the rate table on the "Rates" sheet.
pub fn converted() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"converted\"")
        .with(
            "rate",
            r##"function(self, c, code)
  local rates = c:sheet_named("Rates")
  for h in rates:range(2, 1, 100, 1):cells() do
    if h.value == code then return h:offset(0, 1).value end
  end
  error("no rate for " .. tostring(code) .. " on the Rates sheet")
end"##,
        )
        .with(
            "value",
            r##"local amt = c:at(c.row, 3).value
if type(amt) ~= "number" then return nil end
return amt * self:rate(c, c:at(1, c.col).value)"##,
        )
        .with(
            "display",
            r##"return fmt.thousands(v, 2) .. " " .. c:at(1, c.col).value"##,
        )
        .with(
            "tooltip",
            r##"local code = c:at(1, c.col).value
return string.format("1 AUD = %s %s (from the Rates sheet)", self:rate(c, code), code)"##,
        )
        .with("style", "{ align = \"right\", mono = true, italic = true }")
}

/// Sum of the money cells above, in bold.
pub fn money_total() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"total\"")
        .with("unit", "\"AUD\"")
        .with("value", "return c:above():where_kind(\"money\"):sum()")
        .with(
            "display",
            r##"return { text = fmt.currency(v, "A$", 2), bold = true }"##,
        )
        .with("style", "{ align = \"right\", mono = true }")
}

/// Total of the amounts whose category (column 2) equals the label to the left.
pub fn category_total() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"category total\"")
        .with("unit", "\"AUD\"")
        .with(
            "value",
            r##"local cat = c:left().value
local hits = c.sheet:range(2, 2, 10000, 2):filter(function(h) return h.value == cat end)
local total = 0
for _, h in ipairs(hits) do
  local amt = h:offset(0, 1)
  if amt.kind == "money" then total = total + amt.value end
end
return total"##,
        )
        .with("display", r##"return fmt.currency(v, "A$", 2)"##)
        .with("style", "{ align = \"right\", mono = true }")
}

fn budget(dir: &Path) -> Result<()> {
    let mut store = create(dir, "budget.decel")?;
    let sh = SheetId(1);
    let rates_id = store.next_sheet_id()?;
    let a = |r: u32, c: u32| CellAddr::new(sh, r, c);
    let ra = |r: u32, c: u32| CellAddr::new(rates_id, r, c);
    let (money, bal, pct, fx, total, cat) = (
        aud(),
        balance(),
        share(),
        converted(),
        money_total(),
        category_total(),
    );
    let mut cells = vec![
        (a(0, 0), s("Item")),
        (a(0, 1), s("Category")),
        (a(0, 2), s("Amount")),
        (a(0, 3), s("Balance")),
        (a(0, 4), s("Share")),
        (a(0, 5), s("USD")),
        (a(0, 6), s("EUR")),
        (a(0, 8), s("Category")),
        (a(0, 9), s("Total")),
    ];
    let lines = [
        ("Salary", "Income", 6200.0),
        ("Rent", "Housing", -2150.0),
        ("Groceries", "Food", -640.35),
        ("Electricity", "Utilities", -185.2),
        ("Internet", "Utilities", -79.0),
        ("Fuel", "Transport", -210.5),
        ("Eating out", "Food", -156.8),
        ("Freelance work", "Income", 950.0),
        ("Savings transfer", "Savings", -1500.0),
    ];
    for (i, (item, category, amount)) in lines.iter().enumerate() {
        let r = i as u32 + 1;
        cells.push((a(r, 0), s(item)));
        cells.push((a(r, 1), s(category)));
        cells.push((a(r, 2), num(*amount).with_meta(money.hash())));
        cells.push((a(r, 3), Cell::EMPTY.with_meta(bal.hash())));
        cells.push((a(r, 4), Cell::EMPTY.with_meta(pct.hash())));
        cells.push((a(r, 5), Cell::EMPTY.with_meta(fx.hash())));
        cells.push((a(r, 6), Cell::EMPTY.with_meta(fx.hash())));
    }
    let last = lines.len() as u32 + 1;
    cells.push((a(last, 0), s("Total")));
    cells.push((a(last, 2), Cell::EMPTY.with_meta(total.hash())));
    let categories = [
        "Income",
        "Housing",
        "Food",
        "Utilities",
        "Transport",
        "Savings",
    ];
    for (i, name) in categories.iter().enumerate() {
        let r = i as u32 + 1;
        cells.push((a(r, 8), s(name)));
        cells.push((a(r, 9), Cell::EMPTY.with_meta(cat.hash())));
    }
    // The Rates sheet: units of each currency per Australian dollar.
    cells.extend([
        (ra(0, 0), s("Currency")),
        (ra(0, 1), s("Per AUD")),
        (ra(1, 0), s("USD")),
        (ra(1, 1), num(0.65)),
        (ra(2, 0), s("EUR")),
        (ra(2, 1), num(0.60)),
        (ra(3, 0), s("GBP")),
        (ra(3, 1), num(0.51)),
    ]);
    store.apply(
        TxnInput::new("Budget example")
            .op(Op::RenameSheet {
                id: sh,
                name: "Budget".into(),
            })
            .op(Op::AddSheet {
                name: "Rates".into(),
                position: None,
                id: Some(rates_id),
            })
            .op(Op::SetCells(cells))
            .meta(money)
            .meta(bal)
            .meta(pct)
            .meta(fx)
            .meta(total)
            .meta(cat),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// engineering.decel

/// Finds the value cell (column 2) on the row labelled `label` in column 1.
const INPUT_HELPER: &str = r##"function(self, c, label)
  for h in c.sheet:range(1, 1, 200, 1):cells() do
    if h.value == label then return h:offset(0, 1) end
  end
  error("no row labelled " .. label)
end"##;

/// Sum of the consecutive numbers directly above, warning on mixed units.
pub fn load_sum() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"load sum\"")
        .with("unit", "\"N\"")
        .with(
            "terms",
            r##"function(self, c)
  -- Handles for the unbroken run of numbers directly above this cell.
  local out, n = {}, 1
  while true do
    local h = c:up(n)
    if h.row < 1 or type(h.value) ~= "number" then return out end
    out[#out + 1] = h
    n = n + 1
  end
end"##,
        )
        .with(
            "value",
            r##"local total = 0
for _, h in ipairs(self:terms(c)) do total = total + h.value end
return total"##,
        )
        .with(
            "validate",
            r##"local seen, units = {}, {}
for _, h in ipairs(self:terms(c)) do
  local u = h.unit or "(none)"
  if not seen[u] then seen[u] = true; units[#units + 1] = u end
end
table.sort(units)
if #units > 1 then
  return false, "Mixed units: " .. table.concat(units, ", ")
end
if units[1] and units[1] ~= self.unit then
  return false, "Expected " .. self.unit .. " but the terms are in " .. units[1]
end
return true"##,
        )
        .with(
            "display",
            "return { text = fmt.si(v, self.unit, 4), bold = true }",
        )
        .with("style", "{ align = \"right\" }")
}

pub fn area() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"area\"")
        .with("unit", "\"m²\"")
        .with("input", INPUT_HELPER)
        .with(
            "value",
            "return self:input(c, \"Width\").value * self:input(c, \"Depth\").value",
        )
        .with(
            "validate",
            r##"local w, d = self:input(c, "Width"), self:input(c, "Depth")
if w.unit ~= "m" or d.unit ~= "m" then
  return false, "Width and depth should both be in m (got " ..
    tostring(w.unit) .. " and " .. tostring(d.unit) .. ")"
end
return true"##,
        )
        // fmt.si would print 5 mm² for 0.005 m², which is wrong (1 mm² is
        // 1e-6 m²), so areas use scientific notation instead.
        .with("display", "return fmt.sci(v, 3) .. \" \" .. self.unit")
        .with("style", "{ align = \"right\" }")
}

pub fn stress() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"stress\"")
        .with("unit", "\"Pa\"")
        .with("input", INPUT_HELPER)
        .with(
            "value",
            "return self:input(c, \"Force\").value / self:input(c, \"Area\").value",
        )
        .with(
            "validate",
            r##"local f, a = self:input(c, "Force"), self:input(c, "Area")
if f.unit ~= "N" or a.unit ~= "m²" then
  return false, "Stress needs N and m² (got " ..
    tostring(f.unit) .. " and " .. tostring(a.unit) .. ")"
end
return true"##,
        )
        .with("display", "return fmt.si(v, self.unit, 3)")
        .with(
            "tooltip",
            "return string.format(\"σ = F / A = %.4g Pa\", v)",
        )
        .with("style", "{ align = \"right\" }")
}

pub fn utilisation() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"utilisation\"")
        .with("input", INPUT_HELPER)
        .with(
            "value",
            "return self:input(c, \"Stress\").value / self:input(c, \"Yield strength\").value",
        )
        .with(
            "validate",
            r##"if v > 1 then return false, "Over-stressed: utilisation above 100%" end
return true"##,
        )
        .with(
            "display",
            r##"return {
  text = fmt.percent(v, 1),
  bg = color.scale(v, 0.5, 1, "#d5f5e3", "#f5b7b1"),
  bold = v > 0.9,
}"##,
        )
        .with("style", "{ align = \"right\" }")
}

pub fn safety_factor() -> MetaSource {
    MetaSource::default()
        .with("kind", "\"factor of safety\"")
        .with("value", "return 1 / c:up().value")
        .with("display", "return fmt.fixed(v, 2)")
        .with(
            "validate",
            r##"if v < 1.5 then return false, "Factor of safety below 1.5" end
return true"##,
        )
        .with("style", "{ align = \"right\" }")
}

fn engineering(dir: &Path) -> Result<()> {
    let mut store = create(dir, "engineering.decel")?;
    let sh = SheetId(1);
    let a = |r: u32, c: u32| CellAddr::new(sh, r, c);
    let (load, length, strength) = (
        unit_input("load", "N"),
        unit_input("length", "m"),
        unit_input("strength", "Pa"),
    );
    let (sum, ar, st, ut, fs) = (load_sum(), area(), stress(), utilisation(), safety_factor());
    let rows: Vec<(&str, Cell, &str)> = vec![
        ("Quantity", s("Value"), "Notes"),
        ("Dead load", num(500_000.0).with_meta(load.hash()), "Input"),
        ("Live load", num(250_000.0).with_meta(load.hash()), "Input"),
        (
            "Force",
            Cell::EMPTY.with_meta(sum.hash()),
            "Sum of the loads above",
        ),
        ("Width", num(0.05).with_meta(length.hash()), "Input"),
        ("Depth", num(0.1).with_meta(length.hash()), "Input"),
        ("Area", Cell::EMPTY.with_meta(ar.hash()), "Width × Depth"),
        ("Stress", Cell::EMPTY.with_meta(st.hash()), "Force / Area"),
        (
            "Yield strength",
            num(250e6).with_meta(strength.hash()),
            "Input",
        ),
        (
            "Utilisation",
            Cell::EMPTY.with_meta(ut.hash()),
            "Stress / Yield strength",
        ),
        (
            "Factor of safety",
            Cell::EMPTY.with_meta(fs.hash()),
            "1 / Utilisation",
        ),
        ("", Cell::EMPTY, ""),
        (
            "Mixed-unit check",
            Cell::EMPTY,
            "The sum below shows a warning marker",
        ),
        ("Load A", num(1000.0).with_meta(load.hash()), "Input in N"),
        (
            "Load B",
            num(2.0).with_meta(length.hash()),
            "Tagged m by mistake",
        ),
        (
            "Sum",
            Cell::EMPTY.with_meta(sum.hash()),
            "Hover for the warning",
        ),
    ];
    let mut cells = Vec::new();
    for (i, (label, value, note)) in rows.into_iter().enumerate() {
        let r = i as u32;
        if !label.is_empty() {
            cells.push((a(r, 0), s(label)));
        }
        if value != Cell::EMPTY {
            cells.push((a(r, 1), value));
        }
        if !note.is_empty() {
            cells.push((a(r, 2), s(note)));
        }
    }
    store.apply(
        TxnInput::new("Engineering example")
            .op(Op::RenameSheet {
                id: sh,
                name: "Beam".into(),
            })
            .op(Op::SetCells(cells))
            .meta(load)
            .meta(length)
            .meta(strength)
            .meta(sum)
            .meta(ar)
            .meta(st)
            .meta(ut)
            .meta(fs),
    )?;
    Ok(())
}
