// SPDX-License-Identifier: MIT OR Apache-2.0

//! A resource-limited Lua 5.4 VM.
//!
//! Every evaluation runs under an instruction hook that enforces a wall-clock
//! budget, and the VM has a hard memory cap. Both failures surface as
//! [`VmError`] values rather than panics or aborts.

use std::time::{Duration, Instant};

use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Value, VmState};

/// How often (in VM instructions) the hook checks the deadline.
const HOOK_INSTRUCTION_INTERVAL: u32 = 1_000;

/// Error message used to recognise a budget interrupt raised by the hook.
const TIMEOUT_MARKER: &str = "decel: evaluation budget exceeded";

/// Resource limits for a [`Vm`].
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum wall-clock time for a single [`Vm::eval`] call.
    pub time_budget: Duration,
    /// Maximum bytes the Lua allocator may hold.
    pub memory_limit: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            time_budget: Duration::from_millis(50),
            memory_limit: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VmError {
    #[error("evaluation exceeded its time budget")]
    Timeout,
    #[error("evaluation exceeded the memory limit")]
    Memory,
    #[error("lua error: {0}")]
    Lua(String),
}

/// Classifies an error as a resource-limit failure, if it is one.
fn limit_error(err: &mlua::Error) -> Option<VmError> {
    match err {
        mlua::Error::MemoryError(_) => Some(VmError::Memory),
        // Lua's string library refuses results over INT_MAX before allocating.
        mlua::Error::RuntimeError(msg) if msg.contains("too large") => Some(VmError::Memory),
        mlua::Error::RuntimeError(msg) if msg.contains(TIMEOUT_MARKER) => Some(VmError::Timeout),
        mlua::Error::CallbackError { cause, .. } => limit_error(cause),
        mlua::Error::WithContext { cause, .. } => limit_error(cause),
        _ => None,
    }
}

impl From<mlua::Error> for VmError {
    fn from(err: mlua::Error) -> Self {
        limit_error(&err).unwrap_or_else(|| VmError::Lua(err.to_string()))
    }
}

/// The evaluation deadline, stored as Lua app data so Rust callbacks can see it.
struct Deadline(Instant);

/// Wraps the native `pcall`/`xpcall` so that a caught resource-limit error is
/// re-raised. Ordinary errors keep their original values.
const PROTECTED_CALLS: &str = r#"
local raw_pcall, raw_xpcall, is_limit, error = ...
local function check(ok, ...)
  if not ok and is_limit((...)) then error((...), 0) end
  return ok, ...
end
local function pcall(f, ...)
  return check(raw_pcall(f, ...))
end
local function xpcall(f, handler, ...)
  return check(raw_xpcall(f, function(e)
    if is_limit(e) then return e end
    return handler(e)
  end, ...))
end
return pcall, xpcall
"#;

/// Replaces `pcall` and `xpcall` so that scripts cannot catch (and so
/// ignore) timeout or memory-limit errors.
fn install_protected_calls(lua: &Lua) -> mlua::Result<()> {
    let is_limit = lua.create_function(|lua, err: Value| {
        let past_deadline = lua
            .app_data_ref::<Deadline>()
            .is_some_and(|d| Instant::now() >= d.0);
        let limit = match &err {
            Value::Error(e) => limit_error(e).is_some(),
            Value::String(s) => s.as_bytes().as_ref() == b"not enough memory",
            _ => false,
        };
        Ok(past_deadline || limit)
    })?;
    let globals = lua.globals();
    let (pcall, xpcall): (Function, Function) = lua
        .load(PROTECTED_CALLS)
        .set_name("=decel:protected_calls")
        .call((
            globals.get::<Function>("pcall")?,
            globals.get::<Function>("xpcall")?,
            is_limit,
            globals.get::<Function>("error")?,
        ))?;
    globals.set("pcall", pcall)?;
    globals.set("xpcall", xpcall)?;
    Ok(())
}

/// An owned Lua VM. It is `Send`, so the engine thread can own it.
pub struct Vm {
    lua: Lua,
    limits: Limits,
}

impl Vm {
    pub fn new(limits: Limits) -> Result<Self, VmError> {
        let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        lua.set_memory_limit(limits.memory_limit)?;
        install_protected_calls(&lua)?;
        Ok(Self { lua, limits })
    }

    /// Runs `code` as a chunk and converts its first result to a string.
    pub fn eval(&self, code: &str) -> Result<String, VmError> {
        let deadline = Instant::now() + self.limits.time_budget;
        self.lua.set_app_data(Deadline(deadline));
        self.lua.set_hook(
            HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTION_INTERVAL),
            move |_lua, _debug| {
                if Instant::now() >= deadline {
                    Err(mlua::Error::runtime(TIMEOUT_MARKER))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )?;
        let result = self.lua.load(code).eval::<Value>();
        self.lua.remove_hook();
        self.lua.remove_app_data::<Deadline>();
        let value = result?;
        Ok(value.to_string()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm() -> Vm {
        Vm::new(Limits::default()).unwrap()
    }

    #[test]
    fn evaluates_simple_code() {
        assert_eq!(vm().eval("return 1 + 2").unwrap(), "3");
    }

    #[test]
    fn vm_can_move_to_another_thread() {
        let vm = vm();
        let out = std::thread::spawn(move || vm.eval("return ('ab'):rep(2)"))
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(out, "abab");
    }

    #[test]
    fn infinite_loop_is_interrupted_within_100ms() {
        let vm = vm();
        let start = Instant::now();
        let err = vm.eval("while true do end").unwrap_err();
        let elapsed = start.elapsed();
        assert!(matches!(err, VmError::Timeout), "got {err:?}");
        assert!(elapsed < Duration::from_millis(100), "took {elapsed:?}");
        // The VM is still usable afterwards.
        assert_eq!(vm.eval("return 'ok'").unwrap(), "ok");
    }

    #[test]
    fn pcall_cannot_swallow_timeout_forever() {
        let vm = vm();
        let start = Instant::now();
        let err = vm
            .eval("while true do pcall(function() while true do end end) end")
            .unwrap_err();
        assert!(matches!(err, VmError::Timeout), "got {err:?}");
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn memory_bomb_gives_memory_error() {
        let vm = vm();
        // Over INT_MAX: Lua rejects it before allocating.
        let err = vm.eval(r#"return string.rep("x", 1e10)"#).unwrap_err();
        assert!(matches!(err, VmError::Memory), "got {err:?}");
        // Under INT_MAX but over the VM's cap: the allocator refuses it.
        let err = vm.eval(r#"return string.rep("x", 1e9)"#).unwrap_err();
        assert!(matches!(err, VmError::Memory), "got {err:?}");
        assert_eq!(vm.eval("return 'ok'").unwrap(), "ok");
    }

    #[test]
    fn pcall_cannot_swallow_memory_error() {
        let err = vm()
            .eval(r#"local ok = pcall(string.rep, "x", 1e9) return ok"#)
            .unwrap_err();
        assert!(matches!(err, VmError::Memory), "got {err:?}");
    }

    #[test]
    fn pcall_and_xpcall_still_catch_ordinary_errors() {
        let vm = vm();
        assert_eq!(
            vm.eval("local ok, e = pcall(error, 'boom', 0) return tostring(ok) .. e")
                .unwrap(),
            "falseboom"
        );
        assert_eq!(
            vm.eval("local ok, v = pcall(function(a) return a * 2 end, 21) return v")
                .unwrap(),
            "42"
        );
        assert_eq!(
            vm.eval(
                "local ok, e = xpcall(error, function(m) return 'h:' .. m end, 'x', 0) return e"
            )
            .unwrap(),
            "h:x"
        );
        // Non-string error values survive.
        assert_eq!(
            vm.eval("local ok, e = pcall(error, {code = 7}) return e.code")
                .unwrap(),
            "7"
        );
    }

    #[test]
    fn incremental_memory_growth_gives_memory_error() {
        let vm = vm();
        let err = vm
            .eval("local t = {} for i = 1, 1e9 do t[i] = ('y'):rep(64) .. i end")
            .unwrap_err();
        assert!(
            matches!(err, VmError::Memory | VmError::Timeout),
            "got {err:?}"
        );
    }

    #[test]
    fn unsafe_libraries_are_absent() {
        assert_eq!(
            vm().eval("return tostring(io) .. tostring(os) .. tostring(require)")
                .unwrap(),
            "nilnilnil"
        );
    }
}
