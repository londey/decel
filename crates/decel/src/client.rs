// SPDX-License-Identifier: MIT OR Apache-2.0

//! Bridges the engine thread's event channel into GPUI's async world.

use decel_engine::protocol::Event;
use decel_engine::{EngineHandle, Limits};
use futures::channel::mpsc::{UnboundedReceiver, unbounded};

/// Starts the engine and a forwarding thread. The returned receiver yields
/// engine events on the UI side; it ends when the engine stops.
pub fn start(limits: Limits) -> std::io::Result<(EngineHandle, UnboundedReceiver<Event>)> {
    let handle = EngineHandle::spawn(limits)?;
    let events = handle.events().clone();
    let (tx, rx) = unbounded();
    std::thread::Builder::new()
        .name("decel-events".into())
        .spawn(move || {
            while let Ok(e) = events.recv() {
                if tx.unbounded_send(e).is_err() {
                    break;
                }
            }
        })?;
    Ok((handle, rx))
}
