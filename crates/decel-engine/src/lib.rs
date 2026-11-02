// SPDX-License-Identifier: MIT OR Apache-2.0

//! Headless evaluation engine: Lua host, sandbox, dependency tracking and recalculation.

mod api;
mod bindings;
mod compile;
mod convert;
mod core;
pub mod deps;
mod display;
mod engine;
pub mod sandbox;
pub mod stdlib;
pub mod testkit;

pub use display::{Align, CellStyle, DisplayCell};
pub use engine::{EditRejected, Engine, EngineError, MemberStatus, Preview};
pub use sandbox::Limits;
