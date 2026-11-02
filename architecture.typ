// decel — architecture slides
// Build: typst compile architecture.typ   (no packages required; diagrams are drawn natively)
// Fonts: Inter + JetBrains Mono if installed, falls back to DejaVu.

#let ink = rgb("#1d2230")
#let muted = rgb("#6b7280")
#let faint = rgb("#e7e5df")
#let paper = rgb("#fbfaf7")
#let blue = rgb("#3566c8")
#let teal = rgb("#16917f")
#let amber = rgb("#c98304")
#let rose = rgb("#c63d55")
#let violet = rgb("#6d4fe0")
#let green = rgb("#3a8f47")
#let tint(c) = c.lighten(82%)

#let sans = ("Inter", "DejaVu Sans")
#let mono = ("JetBrains Mono", "DejaVu Sans Mono")

#set page(width: 720pt, height: 405pt, margin: (x: 34pt, top: 58pt, bottom: 26pt), fill: paper)
#set text(font: sans, size: 12.5pt, fill: ink)
#set par(leading: 0.55em)
#show raw: set text(font: mono, size: 9.5pt)
#show raw.where(block: true): it => block(fill: rgb("#f1efe9"), inset: 8pt, radius: 4pt, width: 100%, it)
#set list(marker: text(fill: blue)[▸], spacing: 0.7em)

// ---------- slide frame ----------
#let slide-no = counter("slide")
#let slide(title, section: none, body) = {
  pagebreak(weak: true)
  slide-no.step()
  place(top + left, dx: -34pt, dy: -58pt, rect(width: 720pt, height: 4pt, fill: blue, stroke: none))
  place(top + left, dy: -42pt, { show raw: set text(size: 18pt); text(size: 20pt, weight: "semibold", title) })
  if section != none {
    place(top + right, dy: -38pt, text(size: 9pt, fill: muted, upper(section)))
  }
  place(bottom + right, dy: 16pt, context text(size: 8.5pt, fill: muted)[decel · #slide-no.display()])
  body
}

// ---------- native diagram kit ----------
#let canvas(w, h, body) = box(width: w, height: h, body)
#let nd(x, y, w, h, body, fill: white, stroke: 0.8pt + ink, radius: 4pt, size: 10pt, weight: "regular", inset: 4pt) = place(dx: x, dy: y,
  rect(width: w, height: h, fill: fill, stroke: stroke, radius: radius, inset: inset,
    align(center + horizon, text(size: size, weight: weight, body))))
#let lbl(x, y, body, size: 8.5pt, fill: muted, w: auto) = place(dx: x, dy: y, box(width: w, text(size: size, fill: fill, body)))
#let head(a, b, col) = {
  let dx = (b.at(0) - a.at(0)) / 1pt
  let dy = (b.at(1) - a.at(1)) / 1pt
  let len = calc.sqrt(dx * dx + dy * dy)
  let ux = dx / len
  let uy = dy / len
  let L = 7.0
  let W = 3.6
  let (x2, y2) = b
  place(polygon(fill: col, stroke: none,
    (x2, y2),
    (x2 - (ux * L - uy * W) * 1pt, y2 - (uy * L + ux * W) * 1pt),
    (x2 - (ux * L + uy * W) * 1pt, y2 - (uy * L - ux * W) * 1pt)))
}
// poly-line arrow through points; arrowhead at last point
#let arr(..pts, col: ink, w: 1pt, dash: none, both: false) = {
  let p = pts.pos()
  let st = (paint: col, thickness: w, dash: dash)
  for i in range(p.len() - 1) {
    place(line(start: p.at(i), end: p.at(i + 1), stroke: st))
  }
  head(p.at(p.len() - 2), p.at(p.len() - 1), col)
  if both { head(p.at(1), p.at(0), col) }
}
#let chip(body, col) = box(fill: tint(col), inset: (x: 5pt, y: 2pt), radius: 3pt, text(size: 9pt, fill: col.darken(25%), weight: "medium", body))
// mini grid: cells = array of rows, each entry none | color; text optional via dict
#let minigrid(x, y, rows, cw: 26pt, rh: 15pt, texts: (:), border: none, tinted: true) = {
  for (r, row) in rows.enumerate() {
    for (c, col) in row.enumerate() {
      let f = if col == none or not tinted { white } else { tint(col) }
      let t = texts.at(str(r) + "," + str(c), default: [])
      place(dx: x + c * cw, dy: y + r * rh, rect(width: cw, height: rh, fill: f, stroke: 0.4pt + faint, inset: 1pt,
        align(center + horizon, text(size: 6.5pt, fill: ink, t))))
    }
  }
  // automatic boundaries between differing colours
  if border != none {
    for (r, row) in rows.enumerate() {
      for (c, col) in row.enumerate() {
        if c + 1 < row.len() and row.at(c + 1) != col and (col != none or row.at(c + 1) != none) {
          place(line(start: (x + (c + 1) * cw, y + r * rh), end: (x + (c + 1) * cw, y + (r + 1) * rh), stroke: 1.3pt + border))
        }
        if r + 1 < rows.len() and rows.at(r + 1).at(c) != col and (col != none or rows.at(r + 1).at(c) != none) {
          place(line(start: (x + c * cw, y + (r + 1) * rh), end: (x + (c + 1) * cw, y + (r + 1) * rh), stroke: 1.3pt + border))
        }
      }
    }
  }
}

// =====================================================================
// 1. Title
// =====================================================================
#page(margin: 0pt, fill: ink)[
  #place(top + left, dx: 54pt, dy: 120pt)[
    #text(size: 46pt, weight: "bold", fill: white)[decel]
    #v(-14pt)
    #box(width: 380pt, text(size: 17pt, fill: rgb("#c9cfdd"))[A spreadsheet where every cell can carry a Lua metatable])
    #v(18pt)
    #text(size: 11pt, fill: rgb("#8f97aa"))[Architecture overview · v1.0 · 2 November 2026 · MIT OR Apache-2.0]
  ]
  // decorative tinted grid
  #let C = (blue, teal, amber, rose, violet, green)
  #for r in range(9) {
    for c in range(8) {
      let col = if r < 3 { if c < 4 { none } else { C.at(1) } } else if c < 2 { C.at(0) } else if c < 5 { if r > 6 { C.at(3) } else { C.at(2) } } else { C.at(4) }
      place(dx: 478pt + c * 26pt, dy: 60pt + r * 32pt, rect(width: 25pt, height: 31pt, radius: 2pt, stroke: none,
        fill: if col == none { rgb("#2a3142") } else { col.transparentize(55%) }))
    }
  }
]
#counter("slide").update(1)

// =====================================================================
#slide("One programmable object per cell", section: "Motivation")[
  #grid(columns: (1fr, 1fr), gutter: 24pt,
  [
    Excel spreads a cell's behaviour across many unrelated features. None of them can be seen at a glance, and none of them carries a history.

    #v(6pt)
    decel folds all of them into a *metatable* with *members*:
    - *hooks*: `value`, `display`, `edit`, `parse`, `validate`, `tooltip`, `edit_hint`
    - *fields*: `kind`, `unit`, `style`
    - *helpers*: your own functions

    #v(6pt)
    Then it makes metatables *visible* (peek, code side, boundaries), *shareable* (fill with copy-on-write) and *versioned* (SQLite history).
  ],
  canvas(300pt, 260pt, {
    let items = ([Number format], [Cell style], [Data validation], [Conditional format], [Formula], [Comment / note])
    let cols = (blue, teal, amber, rose, violet, green)
    for (i, it) in items.enumerate() {
      nd(0pt, 8pt + i * 40pt, 118pt, 28pt, it, fill: tint(cols.at(i)), stroke: 0.6pt + cols.at(i), size: 9.5pt)
      arr((120pt, 22pt + i * 40pt), (186pt, 126pt), col: cols.at(i), w: 0.8pt)
    }
    nd(190pt, 96pt, 108pt, 60pt, [*metatable*\ #text(size: 8.5pt, fill: muted)[one Lua object]], fill: white, stroke: 1.4pt + ink, size: 12pt)
  }))
]

// =====================================================================
#slide("The cell model", section: "Concepts")[
  #grid(columns: (330pt, 1fr), gutter: 18pt,
  canvas(330pt, 270pt, {
    nd(110pt, 0pt, 110pt, 34pt, [*Cell* (row, col)], fill: white)
    arr((140pt, 34pt), (60pt, 70pt))
    arr((190pt, 34pt), (270pt, 70pt))
    nd(0pt, 70pt, 120pt, 46pt, [*raw*: Option<Value>\ #text(size: 8pt, fill: muted)[what you typed, parsed]], fill: tint(blue), stroke: 0.7pt + blue)
    nd(210pt, 70pt, 120pt, 46pt, [*meta*: Option<Hash>\ #text(size: 8pt, fill: muted)[BLAKE3 of canonical source]], fill: tint(violet), stroke: 0.7pt + violet)
    arr((270pt, 116pt), (270pt, 146pt), col: violet)
    nd(196pt, 146pt, 134pt, 112pt, align(left)[
      #set text(size: 7.5pt)
      `kind = "latitude"`\
      `unit = "deg"`\
      `value(self, c)`\
      `display(self, c, v)`\
      `edit(self, c, v)`\
      `parse(self, c, txt)`\
      `validate(self, c, v)`\
      `tooltip / edit_hint`
    ], fill: white, stroke: 0.7pt + violet)
    arr((60pt, 116pt), (60pt, 170pt), col: blue)
    nd(0pt, 170pt, 120pt, 40pt, [*final value*\ #text(size: 8pt, fill: muted)[`value()` or raw]], fill: tint(teal), stroke: 0.7pt + teal)
    arr((194pt, 190pt), (122pt, 190pt), col: violet, dash: "dashed")
    lbl(126pt, 196pt, [computes / formats], w: 70pt)
    lbl(0pt, 222pt, [Value = Nil · Bool · Int · Num · Str · Error], size: 8.5pt, w: 200pt)
  }),
  [
    - Both halves are *optional*. A cell with neither is empty and isn't stored.
    - Dates, money and angles are just numbers or strings, given *meaning by the metatable*.
    - The metatable is *content-addressed*. Identical source gives the same hash, the same compiled Lua object and the same colour.
    - Members are stored and edited *one at a time*, REALbasic style.
    - Units are *tags only*. They're displayed and grouped by, but never used in arithmetic.
  ])
]

// =====================================================================
#slide("The value pipeline", section: "Concepts")[
  #canvas(652pt, 190pt, {
    let y = 30pt
    nd(0pt, y, 78pt, 36pt, [typed text], fill: white)
    arr((78pt, y + 18pt), (100pt, y + 18pt))
    nd(100pt, y, 92pt, 36pt, [`parse`\ #text(size: 7.5pt, fill: muted)[or default parser]], fill: tint(blue), stroke: 0.7pt + blue)
    arr((192pt, y + 18pt), (214pt, y + 18pt))
    nd(214pt, y, 80pt, 36pt, [`validate`], fill: tint(amber), stroke: 0.7pt + amber)
    arr((294pt, y + 18pt), (316pt, y + 18pt))
    nd(316pt, y, 78pt, 36pt, [*store raw*\ #text(size: 7.5pt, fill: muted)[txn → SQLite]], fill: white, stroke: 1pt + ink)
    arr((394pt, y + 18pt), (416pt, y + 18pt))
    nd(416pt, y, 80pt, 36pt, [`value`\ #text(size: 7.5pt, fill: muted)[or raw]], fill: tint(violet), stroke: 0.7pt + violet)
    arr((496pt, y + 18pt), (518pt, y + 18pt))
    nd(518pt, y, 134pt, 36pt, [`display` / `tooltip`\ #text(size: 7.5pt, fill: muted)[lazy · visible cells only]], fill: tint(teal), stroke: 0.7pt + teal)
    // reject branch
    arr((254pt, y + 36pt), (254pt, y + 60pt), col: rose)
    nd(194pt, y + 60pt, 120pt, 30pt, [reject edit\ #text(size: 7.5pt)[editor stays open + message]], fill: tint(rose), stroke: 0.7pt + rose, size: 9pt)
    // edit text loop
    arr((585pt, y + 36pt), (585pt, y + 108pt), (40pt, y + 108pt), (40pt, y + 36pt), col: muted, dash: "dashed")
    lbl(330pt, y + 112pt, [`edit(self, c, v)` supplies the text you see when you start editing], w: 300pt)
    // validate on computed
    arr((456pt, y), (456pt, y - 16pt), (254pt, y - 16pt), (254pt, y), col: amber, dash: "dotted")
    lbl(300pt, y - 30pt, [validate also runs on computed values → warning marker, not rejection], w: 330pt)
    lbl(0pt, y + 146pt, [Entering a value is one transaction. Recalculation is derived state and is never written to history.], size: 10pt, fill: ink, w: 600pt)
  })
]

// =====================================================================
#slide("Example: latitude stored as decimal, shown as D°M′S″", section: "Concepts")[
  #grid(columns: (300pt, 1fr), gutter: 18pt,
  canvas(300pt, 270pt, {
    // grid display
    lbl(0pt, 0pt, [*Normal display*], fill: ink, size: 9pt)
    minigrid(0pt, 16pt, ((none, none, none), (none, violet, none), (none, violet, none)), cw: 96pt, rh: 22pt,
      texts: ("0,0": [Site], "0,1": [Lat], "0,2": [Lon], "1,0": [Alpha], "1,1": [35°18′27.0″S], "2,0": [Bravo], "2,1": [35°17′02.4″S]))
    // tooltip
    place(dx: 120pt, dy: 82pt, rect(fill: ink, radius: 3pt, inset: 5pt, text(size: 8pt, fill: white)[−35.307500° — 35°18′27.00″S]))
    lbl(0pt, 86pt, [hover → `tooltip`], w: 110pt)
    // edit bar
    lbl(0pt, 128pt, [*Editing* (F2)], fill: ink, size: 9pt)
    place(dx: 0pt, dy: 144pt, rect(width: 290pt, height: 22pt, fill: white, stroke: 0.8pt + blue, radius: 3pt, inset: 5pt,
      text(size: 9pt)[2,2 │ `-35.307500` #h(1fr) #text(fill: muted, size: 7.5pt)[Decimal degrees or 35°18'27"S]]))
    lbl(0pt, 170pt, [`edit` gives the text · `edit_hint` gives the hint on the right], w: 290pt)
    place(dx: 0pt, dy: 196pt, rect(width: 290pt, height: 22pt, fill: white, stroke: 0.8pt + rose, radius: 3pt, inset: 5pt,
      text(size: 9pt)[2,2 │ `95`]))
    lbl(0pt, 222pt, text(fill: rose)[✗ Latitude must be within ±90° (from `validate`)], w: 290pt)
  }),
  [
    ```lua
    kind = "latitude"
    unit = "deg"
    -- display(self, c, v)
    return fmt.dms(v, "lat", 1)
    -- edit(self, c, v)
    return string.format("%.6f", v)
    -- parse(self, c, text)
    local n = tonumber(text) or parse.dms(text)
    if not n then return nil, "Enter decimal or D°M'S\"" end
    return n
    -- validate(self, c, v)
    if v < -90 or v > 90 then
      return false, "Latitude must be within ±90°"
    end
    return true
    ```
  ])
]

// =====================================================================
#slide([Relative references through the `c` API], section: "Lua API")[
  #grid(columns: (290pt, 1fr), gutter: 22pt,
  canvas(290pt, 270pt, {
    let cw = 40pt
    let rh = 26pt
    let x0 = 20pt
    let y0 = 14pt
    // column header
    for c in range(5) { lbl(x0 + c * cw + 16pt, 0pt, str(c + 1)) }
    for r in range(8) { lbl(4pt, y0 + r * rh + 8pt, str(r + 1)) }
    for r in range(8) { for c in range(5) {
      let f = if c == 2 and r < 5 { tint(teal) } else { white }
      place(dx: x0 + c * cw, dy: y0 + r * rh, rect(width: cw, height: rh, fill: f, stroke: 0.4pt + faint))
    } }
    // range highlight
    place(dx: x0 + 2 * cw, dy: y0, rect(width: cw, height: 5 * rh, stroke: (paint: teal, thickness: 1.4pt, dash: "dashed"), fill: none))
    lbl(x0 + 2 * cw + 4pt, y0 + 2pt, text(fill: teal)[`c:above()`], size: 7.5pt)
    // self
    place(dx: x0 + 2 * cw, dy: y0 + 5 * rh, rect(width: cw, height: rh, fill: tint(violet), stroke: 1.6pt + violet))
    lbl(x0 + 2 * cw + 14pt, y0 + 5 * rh + 8pt, text(fill: violet, weight: "bold")[c], size: 10pt)
    // left
    place(dx: x0 + 1 * cw, dy: y0 + 5 * rh, rect(width: cw, height: rh, fill: tint(amber), stroke: 1pt + amber))
    arr((x0 + 2 * cw + 4pt, y0 + 5.5 * rh), (x0 + 1 * cw + 26pt, y0 + 5.5 * rh), col: amber)
    lbl(x0, y0 + 8 * rh + 6pt, text(fill: amber)[`c:left()` = `c:offset(0,-1)`], size: 7.5pt, w: 180pt)
    // offset down-right
    place(dx: x0 + 4 * cw, dy: y0 + 7 * rh, rect(width: cw, height: rh, fill: tint(rose), stroke: 1pt + rose))
    arr((x0 + 2.5 * cw, y0 + 6 * rh), (x0 + 4.3 * cw, y0 + 7.2 * rh), col: rose)
    lbl(x0 + 3.1 * cw, y0 + 6.2 * rh, text(fill: rose)[`c:offset(2,2)`], size: 7.5pt, w: 80pt)
  }),
  [
    *Integer rows/columns, 1-based. Offsets are relative*, so one metatable works in every row it's filled into.

    ```lua
    c.row, c.col, c.raw, c.kind, c.unit
    c:offset(dr, dc)   c:at(row, col)
    c:up() c:down() c:left() c:right()
    c:above() c:below() c:left_of() c:right_of()
    c:column(r1, r2)   c:row_range(c1, c2)
    c:sheet_named("Inputs"):range(1,1,10,3)
    -- Cell:  h.value h.raw h.ok h.err h.kind
    -- Range: :sum() :avg() :values() :map(f)
    --        :where_kind("latitude")
    ```
    "Rest of column/row" helpers *exclude self*, so a total at the bottom of a column doesn't create a cycle.
  ])
]

// =====================================================================
#slide("Sharing, fill and copy-on-write", section: "Metatables")[
  #canvas(652pt, 290pt, {
    // left: filled column sharing one hash
    lbl(0pt, 0pt, [*Fill down*: 5 cells share one object], fill: ink, size: 9.5pt, w: 200pt)
    for r in range(5) { nd(0pt, 20pt + r * 26pt, 70pt, 22pt, [row #(r + 2)], fill: tint(violet), stroke: 0.6pt + violet, size: 8.5pt) }
    for r in range(5) { arr((70pt, 31pt + r * 26pt), (118pt, 84pt), col: violet, w: 0.7pt) }
    nd(120pt, 66pt, 82pt, 36pt, [hash `a1f3…`\ #text(size: 7.5pt)[frozen Lua table]], fill: white, stroke: 1.2pt + violet, size: 9pt)
    // middle: COW
    lbl(232pt, 0pt, [*Plain click → COW edit*], fill: ink, size: 9.5pt, w: 200pt)
    for r in range(5) {
      let c = if r == 2 { amber } else { violet }
      nd(232pt, 20pt + r * 26pt, 70pt, 22pt, [row #(r + 2)], fill: tint(c), stroke: 0.6pt + c, size: 8.5pt)
    }
    for r in (0, 1, 3, 4) { arr((302pt, 31pt + r * 26pt), (346pt, 70pt), col: violet, w: 0.7pt) }
    arr((302pt, 83pt), (346pt, 128pt), col: amber, w: 1pt)
    nd(348pt, 52pt, 70pt, 30pt, [`a1f3…`], fill: white, stroke: 1pt + violet, size: 9pt)
    nd(348pt, 116pt, 70pt, 30pt, [`7c02…` new], fill: white, stroke: 1pt + amber, size: 9pt)
    // right: shared
    lbl(452pt, 0pt, [*Shift + click → shared edit*], fill: ink, size: 9.5pt, w: 200pt)
    for r in range(5) { nd(452pt, 20pt + r * 26pt, 70pt, 22pt, [row #(r + 2)], fill: tint(teal), stroke: 1.4pt + teal, size: 8.5pt) }
    for r in range(5) { arr((522pt, 31pt + r * 26pt), (570pt, 84pt), col: teal, w: 0.7pt) }
    nd(572pt, 66pt, 80pt, 36pt, [`e9b4…`\ #text(size: 7.5pt)[all repointed]], fill: white, stroke: 1.2pt + teal, size: 9pt)
    lbl(452pt, 152pt, [hover with Shift outlines the share set and shows its count, including other sheets], w: 200pt)
    // bottom notes
    place(dx: 0pt, dy: 200pt, block(width: 652pt, text(size: 10.5pt)[
      - *Share set* = every cell with the same hash. Identity is *structural*, so cells with identical source are always shared.
      - *Auto-fill column/row* (`Ctrl+Shift+D/R`) replaces the adjacent run of repeating metatables with the selected one, after a preview.
      - Edits commit on blur or `Ctrl+Enter`. One commit is one undoable transaction, however many cells it repoints.
    ]))
  })
]

// =====================================================================
#slide("Seeing metatables: peek, boundaries, code side", section: "UI")[
  #let rows = (
    (none, none, none, none, none),
    (blue, violet, violet, amber, none),
    (blue, violet, violet, amber, none),
    (blue, violet, violet, amber, none),
    (none, none, none, teal, none),
  )
  #let txt-normal = ("0,0": [Site], "0,1": [Lat], "0,2": [Lon], "0,3": [Elev],
    "1,0": [Alpha], "1,1": [35°18′S], "1,2": [149°7′E], "1,3": [577 m],
    "2,0": [Bravo], "2,1": [35°17′S], "2,2": [149°8′E], "2,3": [581 m],
    "3,0": [Delta], "3,1": [35°16′S], "3,2": [149°6′E], "3,3": [602 m],
    "4,3": [Σ 1760 m])
  #let txt-code = ("1,0": [site], "1,1": [latitude], "1,2": [latitude], "1,3": [elev],
    "2,0": [site], "2,1": [latitude], "2,2": [latitude], "2,3": [elev],
    "3,0": [site], "3,1": [latitude], "3,2": [latitude], "3,3": [elev], "4,3": [total])
  #canvas(652pt, 270pt, {
    lbl(0pt, 0pt, [*Normal* · boundaries drawn where the hash changes], fill: ink, size: 9pt, w: 210pt)
    minigrid(0pt, 30pt, rows, cw: 40pt, rh: 20pt, texts: txt-normal, border: muted, tinted: false)

    lbl(222pt, 0pt, [*Peek* · hold backtick → tint + legend], fill: ink, size: 9pt, w: 210pt)
    minigrid(222pt, 30pt, rows, cw: 40pt, rh: 20pt, texts: txt-normal, border: ink)
    place(dx: 222pt, dy: 136pt, rect(fill: white, stroke: 0.6pt + faint, radius: 3pt, inset: 5pt, text(size: 7pt)[
      #box(width: 6pt, height: 6pt, fill: violet) latitude · deg · 6\
      #box(width: 6pt, height: 6pt, fill: amber) elev · m · 3\
      #box(width: 6pt, height: 6pt, fill: blue) site · 3\
      #box(width: 6pt, height: 6pt, fill: teal) total · m · 1]))

    lbl(444pt, 0pt, [*Code side* · `F3` flips the grid], fill: ink, size: 9pt, w: 210pt)
    minigrid(444pt, 30pt, rows, cw: 40pt, rh: 20pt, texts: txt-code, border: ink)
    place(dx: 484pt, dy: 50pt, rect(width: 80pt, height: 60pt, stroke: (paint: rose, thickness: 1.6pt, dash: "dashed"), fill: none))
    lbl(484pt, 136pt, text(fill: rose)[Shift-hover: share set], w: 120pt)
    place(dx: 0pt, dy: 196pt, block(width: 652pt, text(size: 10pt)[
      - Colours come from a 16-hue OKLCH palette (light and dark), assigned in order of first appearance and stored in the workbook so they stay stable.
      - Boundaries are a 1.5 px line between adjacent cells whose hashes differ. They're neutral normally and coloured in peek and the code side.
      - Clicking a cell in the code side opens its metatable in the dock: the member list on the left, one member's body on the right.
    ]))
  })
]

// =====================================================================
#slide("Metatable editor: one member at a time", section: "UI")[
  #grid(columns: (330pt, 1fr), gutter: 20pt,
  canvas(330pt, 270pt, {
    place(rect(width: 330pt, height: 262pt, fill: white, stroke: 0.8pt + ink, radius: 5pt))
    place(dx: 0pt, dy: 0pt, rect(width: 330pt, height: 22pt, fill: tint(amber), stroke: none, radius: (top: 5pt)))
    lbl(8pt, 6pt, text(fill: amber.darken(30%))[Editing this cell only · shared by 312 others], size: 8.5pt, w: 300pt)
    // member list
    let ms = (([kind], [✓]), ([unit], [✓]), ([value], [✓]), ([*display*], [✓]), ([edit], [✓]), ([parse], [✓]), ([validate], [⚠]), ([tooltip], [✓]), ([\+ add hook], []), ([\+ helper], []))
    for (i, m) in ms.enumerate() {
      let f = if i == 3 { tint(blue) } else { white }
      place(dx: 6pt, dy: 28pt + i * 22pt, rect(width: 96pt, height: 20pt, fill: f, stroke: none, inset: 4pt, text(size: 8.5pt)[#m.at(0) #h(1fr) #text(fill: if m.at(1) == [⚠] { amber } else { green })[#m.at(1)]]))
    }
    place(dx: 106pt, dy: 28pt, line(length: 226pt, angle: 90deg, stroke: 0.5pt + faint))
    place(dx: 112pt, dy: 30pt, rect(width: 210pt, height: 18pt, fill: rgb("#f1efe9"), stroke: none, inset: 4pt, text(size: 8pt, font: mono, fill: muted)[function display(self, c, v)]))
    place(dx: 112pt, dy: 52pt, block(width: 210pt, text(size: 8.5pt, font: mono)[
      #text(fill: violet)[return] fmt.dms(v, #text(fill: green)["lat"], 1)
    ]))
    place(dx: 112pt, dy: 140pt, rect(width: 210pt, height: 18pt, fill: rgb("#f1efe9"), stroke: none, inset: 4pt, text(size: 8pt, font: mono, fill: muted)[end]))
    place(dx: 112pt, dy: 170pt, line(length: 210pt, stroke: 0.5pt + faint))
    lbl(114pt, 178pt, text(fill: green)[✓ compiles], size: 8.5pt, w: 200pt)
    lbl(114pt, 194pt, [preview on 2,2: #text(fill: ink)[35°18′27.0″S]], size: 8.5pt, w: 200pt)
    lbl(114pt, 220pt, [log: (print output)], size: 8pt, w: 200pt)
  }),
  [
    - Like REALbasic's method list: a metatable is a *list of members*, not one long file.
    - Hooks have a *fixed signature header*. You only edit the body.
    - *Per-member* compile status. Runtime errors are flagged against the selected cell.
    - A *live preview* evaluates the uncommitted edit against the selected cell in a scratch context.
    - Code editing uses gpui-component's editor with `tree-sitter-lua` highlighting.
    - Commit on blur or `Ctrl+Enter`, revert with `Esc`. Uncommitted text survives switching members.
  ])
]

// =====================================================================
#slide("Crate architecture", section: "Architecture")[
  #grid(columns: (380pt, 1fr), gutter: 18pt,
  canvas(380pt, 280pt, {
    nd(60pt, 0pt, 230pt, 50pt, [*decel* (binary)\ #text(size: 8pt)[GPUI app · grid element · dock · editor · history · charts · palette · config]], fill: tint(blue), stroke: 1pt + blue)
    arr((175pt, 50pt), (175pt, 76pt))
    nd(60pt, 76pt, 230pt, 50pt, [*decel-engine*\ #text(size: 8pt)[Lua host · sandbox · stdlib · compile cache · dep graph · recalc · actor]], fill: tint(violet), stroke: 1pt + violet)
    arr((175pt, 126pt), (175pt, 152pt))
    nd(60pt, 152pt, 230pt, 44pt, [*decel-store*\ #text(size: 8pt)[SQLite schema · txns · undo/redo · history · lock]], fill: tint(amber), stroke: 1pt + amber)
    arr((175pt, 196pt), (175pt, 222pt))
    nd(60pt, 222pt, 230pt, 44pt, [*decel-model*\ #text(size: 8pt)[Value · Cell · MetaSource/Hash · Sheet · Ops]], fill: tint(teal), stroke: 1pt + teal)
    nd(300pt, 152pt, 80pt, 44pt, [*decel-io*\ #text(size: 8pt)[CSV]], fill: white, stroke: 0.8pt + ink)
    arr((340pt, 196pt), (292pt, 240pt))
    arr((60pt, 100pt), (30pt, 100pt), (30pt, 244pt), (58pt, 244pt), col: muted, dash: "dashed")
    lbl(0pt, 160pt, [uses], w: 28pt)
    // GPUI boundary
    place(dx: 50pt, dy: 63pt, line(length: 250pt, stroke: (paint: rose, thickness: 1pt, dash: "dashed")))
    lbl(300pt, 56pt, text(fill: rose)[GPUI boundary], w: 80pt)
    lbl(300pt, 76pt, [headless below:\ CLI / web later], w: 80pt)
  }),
  [
    *Key dependencies*
    #v(2pt)
    #set text(size: 10.5pt)
    - `gpui-component` 0.8 + `gpui-pre` (exact pin)
    - `mlua` 0.12: `lua54`, `vendored`, `send`
    - `rusqlite` 0.41: `bundled`
    - `blake3`: metatable hashes
    - `rstar`: range-dependency index
    - `crossbeam-channel`, `csv`, `toml`, `directories`
    - tests: `proptest`, `insta`, `criterion`

    #v(4pt)
    All crates are published in lockstep, so `cargo install decel` works. There are *no git dependencies*.
  ])
]

// =====================================================================
#slide("Threads and the engine protocol", section: "Architecture")[
  #canvas(652pt, 290pt, {
    // UI thread lane
    nd(0pt, 0pt, 230pt, 210pt, [], fill: tint(blue), stroke: 1pt + blue, radius: 6pt)
    lbl(10pt, 8pt, text(fill: blue.darken(20%), weight: "bold")[UI thread (GPUI)], size: 10pt, w: 200pt)
    nd(16pt, 34pt, 198pt, 30pt, [GridView · Dock · Editor · Charts], fill: white, size: 9pt)
    nd(16pt, 76pt, 198pt, 42pt, [*ViewportCache*\ #text(size: 7.5pt)[DisplayCell for visible rect + margin]], fill: white, size: 9pt)
    nd(16pt, 130pt, 198pt, 30pt, [optimistic edits, reconciled by seq], fill: white, size: 8.5pt)
    nd(16pt, 170pt, 198pt, 30pt, [never touches Lua or SQLite], fill: white, size: 8.5pt, stroke: 0.8pt + rose)
    // engine lane
    nd(422pt, 0pt, 230pt, 210pt, [], fill: tint(violet), stroke: 1pt + violet, radius: 6pt)
    lbl(432pt, 8pt, text(fill: violet.darken(20%), weight: "bold")[Engine thread], size: 10pt, w: 200pt)
    nd(438pt, 34pt, 198pt, 30pt, [`mlua::Lua` + compile cache], fill: white, size: 9pt)
    nd(438pt, 72pt, 198pt, 30pt, [Model + DepGraph + R-tree], fill: white, size: 9pt)
    nd(438pt, 110pt, 198pt, 30pt, [Store (`rusqlite`, WAL)], fill: white, size: 9pt)
    nd(438pt, 150pt, 198pt, 44pt, [recalc in ~5 ms slices\ #text(size: 7.5pt)[pre-empted by new commands]], fill: white, size: 9pt)
    // channels
    arr((232pt, 60pt), (420pt, 60pt), col: ink, w: 1.3pt)
    lbl(258pt, 34pt, [*Command* (seq)], fill: ink, size: 9pt, w: 160pt)
    lbl(244pt, 66pt, [EnterText · SetMeta{Cow|Shared} · Fill · AutoFill · Paste · Undo · Redo · Restore · Subscribe(rect) · Preview · Query…], size: 7.5pt, w: 170pt)
    arr((420pt, 160pt), (232pt, 160pt), col: violet, w: 1.3pt)
    lbl(258pt, 134pt, text(fill: violet)[*Event*], size: 9pt, w: 160pt)
    lbl(244pt, 166pt, [Ack · CellsUpdated(rect) · TxnCommitted · RecalcProgress · HistoryResult · ShareSetResult · Error], size: 7.5pt, w: 170pt)
    lbl(276pt, 108pt, [crossbeam channels], size: 8pt, w: 120pt)
    place(dx: 0pt, dy: 226pt, block(width: 652pt, text(size: 10.5pt)[
      The engine is the *single source of truth*. The UI only renders what it subscribed to, so scrolling a million-row sheet means re-subscribing to a rectangle, not copying data.
    ]))
  })
]

// =====================================================================
#slide("Dependencies are recorded while code runs", section: "Engine")[
  #grid(columns: (310pt, 1fr), gutter: 20pt,
  canvas(310pt, 270pt, {
    nd(0pt, 10pt, 60pt, 28pt, [2,1], fill: tint(blue), stroke: 0.7pt + blue, size: 9pt)
    nd(0pt, 60pt, 60pt, 28pt, [3,1], fill: tint(blue), stroke: 0.7pt + blue, size: 9pt)
    nd(0pt, 110pt, 60pt, 28pt, [4,1], fill: tint(blue), stroke: 0.7pt + blue, size: 9pt)
    nd(120pt, 60pt, 70pt, 28pt, [3,2 `value`], fill: tint(violet), stroke: 0.7pt + violet, size: 9pt)
    arr((60pt, 74pt), (118pt, 74pt), col: violet)
    lbl(66pt, 58pt, [`c:left()`], size: 7.5pt)
    arr((155pt, 60pt), (155pt, 24pt), (62pt, 24pt), col: violet)
    lbl(80pt, 8pt, [`c:offset(-1,-1)`], size: 7.5pt, w: 100pt)
    // range dep
    place(dx: -4pt, dy: 4pt, rect(width: 68pt, height: 140pt, stroke: (paint: teal, thickness: 1.2pt, dash: "dashed"), fill: none))
    nd(230pt, 130pt, 76pt, 30pt, [6,1 total], fill: tint(teal), stroke: 0.7pt + teal, size: 9pt)
    arr((64pt, 140pt), (228pt, 145pt), col: teal)
    lbl(80pt, 160pt, text(fill: teal)[range dep `(sheet, rect)` → R-tree], size: 7.5pt, w: 160pt)
    nd(0pt, 190pt, 306pt, 68pt, align(left)[
      #set text(size: 8.5pt)
      *edge store*: forward and reverse maps for cell → cell\
      *R-tree*: rect → dependents (point query on change)\
      *name deps*: `sheet_named("Inputs")` → cells\
      *display graph*: kept separate, only invalidates rendering
    ], fill: white, stroke: 0.6pt + faint)
  }),
  [
    *Recalculation*
    + An edit txn marks the changed cells.
    + *Dirty marking* is a breadth-first walk over edges, R-tree hits and name hits.
    + *Demand-driven evaluation*: reading a dirty input evaluates it first. An in-progress stack catches cycles, giving `#CYCLE`.
    + Dependencies are *replaced* on every evaluation, so conditional reads stay correct.
    + Each call has an *instruction budget* (`#TIMEOUT`) and the VM has a *memory cap* (`#MEMORY`).
    + Results are pushed only for subscribed rectangles.

    #v(4pt)
    Verified by a *differential fuzz test*: incremental recalc must equal a full recalc.
  ])
]

// =====================================================================
#slide("Storage: one SQLite file, append-only history", section: "Persistence")[
  #canvas(652pt, 290pt, {
    let tbl(x, y, name, cols, col) = {
      nd(x, y, 140pt, 18pt, text(fill: white, weight: "bold")[#name], fill: col, stroke: 0.8pt + col, radius: 3pt, size: 8.5pt)
      place(dx: x, dy: y + 18pt, rect(width: 140pt, height: cols.len() * 12pt + 6pt, fill: white, stroke: 0.8pt + col, inset: 4pt,
        text(size: 7.5pt, font: mono, cols.join(linebreak()))))
    }
    lbl(0pt, 0pt, [*current state* (materialised)], fill: ink, size: 9pt, w: 300pt)
    tbl(0pt, 16pt, "cells", ("sheet_id, row, col  PK", "vtype, value", "meta → metatables"), teal)
    tbl(160pt, 16pt, "metatables", ("hash  PK (blake3/16)", "canonical", "created_txn"), violet)
    tbl(0pt, 100pt, "sheets", ("id, name, position",), teal)
    tbl(160pt, 100pt, "layout · charts · workbook", ("widths/heights", "chart spec JSON", "settings, colours"), teal)
    arr((140pt, 52pt), (158pt, 52pt), col: violet)
    lbl(360pt, 0pt, [*history* (append-only)], fill: ink, size: 9pt, w: 300pt)
    tbl(360pt, 16pt, "txns", ("id, ts, label", "kind: normal|undo|redo|restore", "target → txns"), amber)
    tbl(510pt, 16pt, "cell_changes", ("sheet_id,row,col,txn PK", "before (vtype,value,meta)", "after  (vtype,value,meta)"), amber)
    tbl(510pt, 100pt, "other_changes", ("txn, seq, entity", "before / after JSON"), amber)
    arr((508pt, 50pt), (502pt, 50pt), col: amber)
    arr((508pt, 125pt), (430pt, 125pt), (430pt, 84pt), col: amber)
    // timeline
    lbl(0pt, 176pt, [*Undo is a new transaction*. Nothing is ever deleted, and the undo stack is rebuilt from `txns` on open.], fill: ink, size: 9.5pt, w: 640pt)
    let ts = (([t1 edit], white), ([t2 fill 500], white), ([t3 meta], white), ([t4 undo→t3], tint(rose)), ([t5 undo→t2], tint(rose)), ([t6 redo→t2], tint(green)), ([t7 edit], white))
    for (i, t) in ts.enumerate() {
      nd(i * 93pt, 200pt, 84pt, 28pt, t.at(0), fill: t.at(1), stroke: 0.7pt + ink, size: 8.5pt)
      if i > 0 { arr((i * 93pt - 9pt, 214pt), (i * 93pt - 1pt, 214pt)) }
    }
    arr((3 * 93pt + 42pt, 228pt), (3 * 93pt + 42pt, 244pt), (2 * 93pt + 42pt, 244pt), (2 * 93pt + 42pt, 230pt), col: rose, dash: "dashed")
    arr((4 * 93pt + 42pt, 228pt), (4 * 93pt + 42pt, 256pt), (1 * 93pt + 42pt, 256pt), (1 * 93pt + 42pt, 230pt), col: rose, dash: "dashed")
    lbl(0pt, 266pt, [WAL mode · every txn commits immediately (there's no Save) · Save As = `VACUUM INTO` · a lock file prevents two writers · per-cell history is a primary-key range scan], size: 8.5pt, w: 652pt)
  })
]

// =====================================================================
#slide("Distribution, platforms and licence", section: "Release")[
  #grid(columns: (1fr, 1fr), gutter: 24pt,
  [
    *Install*
    ```sh
    cargo install decel
    ```
    - Lua 5.4 and SQLite are *vendored*. The only system needs are a C toolchain and GPUI's platform libraries.
    - Linux needs `xkbcommon`, `wayland`, `xcb`, `fontconfig`, `freetype` and the `vulkan` loader. Windows needs the MSVC build tools.
    - CI runs `cargo publish --workspace --dry-run` to make sure there are no git dependencies.

    *Platforms for v1.0*: Linux (Wayland and X11) and Windows. macOS and web come later.
  ],
  [
    *Licence*: `MIT OR Apache-2.0`
    - `LICENSE-MIT` and `LICENSE-APACHE` at the root, and an SPDX header in every file.
    - `cargo-deny` enforces dependency licences and bans git sources.

    *CI* (Forgejo Actions)
    #v(2pt)
    #canvas(300pt, 70pt, {
      let steps = ([fmt], [clippy], [test], [deny], [publish\ dry-run])
      for (i, s) in steps.enumerate() {
        nd(i * 60pt, 6pt, 52pt, 30pt, s, fill: tint(green), stroke: 0.6pt + green, size: 8.5pt)
        if i > 0 { arr((i * 60pt - 8pt, 21pt), (i * 60pt - 1pt, 21pt), col: green) }
      }
      lbl(0pt, 44pt, [matrix: Linux runner + self-hosted Windows runner (Legion)], size: 8.5pt, w: 300pt)
    })
  ])
]

// =====================================================================
#slide("Roadmap to v1.0", section: "Plan")[
  #canvas(652pt, 290pt, {
    let ph = (
      ("0", [Foundations & spikes], 0, 1, rose),
      ("1", [Model & store], 1, 1, amber),
      ("2", [Lua engine], 2, 2, violet),
      ("3", [Engine actor], 4, 1, violet),
      ("4", [Grid UI], 5, 2, blue),
      ("5", [Metatable UX], 7, 2, blue),
      ("6", [History UI], 9, 1, teal),
      ("7", [Sheets & structure], 10, 1, teal),
      ("8", [CSV], 11, 1, green),
      ("9", [Charts], 11, 2, green),
      ("10", [Polish & release 1.0], 13, 1, ink),
    )
    let u = 34pt
    let rh = 21pt
    let x0 = 150pt
    for (i, p) in ph.enumerate() {
      let (n, name, s, d, col) = p
      let y = i * rh
      lbl(0pt, y + 4pt, [*#n* #h(4pt) #name], fill: ink, size: 9pt, w: 148pt)
      place(dx: x0 + s * u, dy: y + 2pt, rect(width: d * u - 3pt, height: rh - 6pt, fill: tint(col), stroke: 0.7pt + col, radius: 3pt))
      place(dx: 0pt, dy: y + rh - 1pt, line(length: 652pt, stroke: 0.3pt + faint))
    }
    lbl(x0 + 12 * u + 4pt, 8 * rh + 3pt, text(fill: green)[8 ∥ 9 in parallel], size: 7.5pt, w: 90pt)
    lbl(0pt, 11 * rh + 12pt, [Relative effort units, not calendar weeks. Each phase ends with green CI, acceptance criteria checked and a `phase-N` tag.], size: 9pt, w: 640pt)
  })
]

// =====================================================================
#slide("Top risks", section: "Plan")[
  #set text(size: 11pt)
  #table(columns: (170pt, 60pt, 1fr), stroke: none, inset: (x: 6pt, y: 7pt),
    fill: (_, r) => if r == 0 { tint(blue) } else if calc.odd(r) { white } else { rgb("#f4f2ec") },
    [*Risk*], [*Impact*], [*Mitigation*],
    [GPUI / gpui-component churn or publishing gaps], text(fill: rose)[High], [Exact pins. Phase 0 proves `cargo install` on Linux and Windows. Thin UI layer. Fallback is plain `gpui` plus a minimal editor widget.],
    [Recorded dependencies miss a read], text(fill: rose)[High], [All access goes through userdata handles. A differential fuzz test checks incremental against full recalc.],
    [Runaway Lua], text(fill: amber)[Medium], [Instruction hook, memory cap, pre-emptible recalc slices],
    [Windows GPU or driver quirks], text(fill: amber)[Medium], [Test on the Legion laptop in Phase 0 and in every UI phase],
    [History growth], text(fill: green)[Low], [Before/after images only. Compaction is post-v1.],
  )
]
