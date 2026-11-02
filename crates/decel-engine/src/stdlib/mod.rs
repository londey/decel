// SPDX-License-Identifier: MIT OR Apache-2.0

//! Pure-Rust implementations of the decel sandbox modules `fmt`, `parse` and `color`
//! (DESIGN §4.5). The Lua bindings live elsewhere; nothing here depends on mlua.

pub mod color;
pub mod fmt;
pub mod parse;
