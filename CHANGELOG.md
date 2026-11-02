# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
