// SPDX-License-Identifier: MIT OR Apache-2.0

//! Headless evaluation engine: Lua host, sandbox, dependency tracking and recalculation.

pub mod actor;
mod api;
mod bindings;
mod compile;
mod convert;
mod core;
pub mod deps;
mod display;
mod engine;
pub mod fill;
pub mod protocol;
pub mod sandbox;
pub mod stdlib;
pub mod testkit;

pub use actor::{CommandError, EngineHandle};
pub use display::{Align, CellStyle, DisplayCell};
pub use engine::{EditRejected, Engine, EngineError, MemberStatus, Preview};
pub use sandbox::Limits;
