# decel — notes for Claude Code
- Spec: DESIGN.md (source of truth). Plan: IMPLEMENTATION_PLAN.md. Work one phase at a time.
- Crates: decel-model (pure types) → decel-store (SQLite) → decel-engine (Lua, deps, recalc, actor) → decel (GPUI app). decel-io: CSV.
- Only `decel` may depend on GPUI. The engine must stay headless.
- Lua: mlua lua54+vendored+send. Sandbox per DESIGN §4.5 — never expose io/os/load/require.
- Rows/cols are 1-based in Lua and UI, 0-based in Rust (`CellAddr`). Convert only at the Lua/UI boundary.
- Every mutation = one txn through decel-store; never write SQLite elsewhere.
- Before finishing a task: cargo fmt; cargo clippy --workspace --all-targets -D warnings; cargo nextest run --workspace; cargo deny check. (`just check` runs all of these.)
- No git dependencies. No unwrap/expect outside tests.
- If you change behaviour described in DESIGN.md, update DESIGN.md in the same change.
