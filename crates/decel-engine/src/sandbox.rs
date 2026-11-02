// SPDX-License-Identifier: MIT OR Apache-2.0

//! The Lua sandbox (DESIGN §4.5).
//!
//! One Lua 5.4 VM with:
//! - a frozen, read-only global environment containing only the safe base
//!   functions, `string`, `table`, `math` (without `random`), `utf8` and
//!   decel's own modules;
//! - a per-evaluation instruction budget (plus a wall-clock safety net)
//!   enforced by a hook;
//! - a hard memory cap;
//! - `pcall`/`xpcall` wrappers that cannot catch interrupts (timeouts,
//!   memory errors, and the engine's "evaluate this dependency first"
//!   signal);
//! - `print` redirected to a per-evaluation log.

use std::time::{Duration, Instant};

use decel_model::{CellAddr, CellError, ErrorKind};
use mlua::{
    Function, HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value as LuaValue, VmState,
};

/// How often (in VM instructions) the hook checks the budget.
const HOOK_INTERVAL: u32 = 1_000;

/// Maximum lines kept in the per-evaluation `print` log.
const LOG_LINES: usize = 200;

/// Resource limits (DESIGN §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// VM instructions allowed per hook call.
    pub instructions: u64,
    /// Wall-clock cap per hook call, as a safety net.
    pub time: Duration,
    /// Bytes the Lua allocator may hold.
    pub memory: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            instructions: 10_000_000,
            time: Duration::from_millis(500),
            memory: 512 * 1024 * 1024,
        }
    }
}

/// Signals raised from Rust into Lua that scripts must not be able to catch.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Interrupt {
    #[error("evaluation budget exceeded")]
    Timeout,
    /// The evaluation read a dependency that must be evaluated first.
    #[error("dependency {0} must be evaluated first")]
    NeedDep(CellAddr),
}

/// A cell error being re-raised because a hook read an error cell.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct Propagated(pub CellError);

/// What went wrong in a Lua call.
#[derive(Debug, Clone)]
pub enum Fault {
    NeedDep(CellAddr),
    Error(CellError),
}

/// Classifies an interrupt hidden anywhere in an error chain.
fn interrupt(err: &mlua::Error) -> Option<Fault> {
    if let Some(i) = err.downcast_ref::<Interrupt>() {
        return Some(match i {
            Interrupt::Timeout => Fault::Error(CellError::new(
                ErrorKind::Timeout,
                "evaluation exceeded its instruction budget",
            )),
            Interrupt::NeedDep(a) => Fault::NeedDep(*a),
        });
    }
    match err {
        mlua::Error::MemoryError(_) => Some(Fault::Error(CellError::new(
            ErrorKind::Memory,
            "evaluation exceeded the memory limit",
        ))),
        mlua::Error::RuntimeError(msg) if msg.contains("too large") => Some(Fault::Error(
            CellError::new(ErrorKind::Memory, "result too large"),
        )),
        mlua::Error::CallbackError { cause, .. } | mlua::Error::WithContext { cause, .. } => {
            interrupt(cause)
        }
        _ => None,
    }
}

/// Turns any error from a hook call into a [`Fault`], attributing Lua errors
/// to `member` and the line reported by Lua.
pub fn classify(err: &mlua::Error, member: &str) -> Fault {
    if let Some(f) = interrupt(err) {
        return f;
    }
    if let Some(Propagated(e)) = err.downcast_ref::<Propagated>() {
        return Fault::Error(e.clone());
    }
    let (message, line) = lua_message(err);
    let (member_name, line, message) = split_location(&message).map_or(
        (member.to_string(), line, message.clone()),
        |(m, l, rest)| (m, Some(l), rest),
    );
    Fault::Error(CellError {
        kind: ErrorKind::Lua,
        message: message.into(),
        member: Some(member_name.into()),
        line,
    })
}

/// The innermost human-readable message, without traceback.
fn lua_message(err: &mlua::Error) -> (String, Option<u32>) {
    match err {
        mlua::Error::RuntimeError(m) | mlua::Error::SyntaxError { message: m, .. } => {
            let m = m.split("\nstack traceback:").next().unwrap_or(m);
            (m.trim().to_string(), None)
        }
        mlua::Error::CallbackError { cause, .. } | mlua::Error::WithContext { cause, .. } => {
            lua_message(cause)
        }
        mlua::Error::ExternalError(e) => (e.to_string(), None),
        mlua::Error::BadArgument { pos, cause, .. } => {
            let (m, l) = lua_message(cause);
            (format!("bad argument #{pos}: {m}"), l)
        }
        other => (other.to_string(), None),
    }
}

/// Splits `"[string \"x\"]:3: msg"` / `"member:3: msg"` into parts.
fn split_location(msg: &str) -> Option<(String, u32, String)> {
    let msg = msg
        .strip_prefix("runtime error: ")
        .or_else(|| msg.strip_prefix("syntax error: "))
        .unwrap_or(msg);
    let (name, rest) = msg.split_once(':')?;
    let (line, rest) = rest.split_once(':')?;
    let line: u32 = line.trim().parse().ok()?;
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    Some((name.to_string(), line, rest.trim().to_string()))
}

/// Per-evaluation instruction budget, kept in Lua app data for the hook.
struct Budget {
    used: u64,
    limit: u64,
    deadline: Option<Instant>,
}

impl Budget {
    fn exceeded(&self) -> bool {
        self.used > self.limit || self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

/// The `print` log for the current evaluation.
#[derive(Default)]
struct Log(Vec<String>);

/// Wraps native `pcall`/`xpcall` so a caught interrupt is re-raised.
const PROTECTED_CALLS: &str = r#"
local raw_pcall, raw_xpcall, is_interrupt, error = ...
local function check(ok, ...)
  if not ok and is_interrupt((...)) then error((...), 0) end
  return ok, ...
end
local function pcall(f, ...)
  return check(raw_pcall(f, ...))
end
local function xpcall(f, handler, ...)
  return check(raw_xpcall(f, function(e)
    if is_interrupt(e) then return e end
    return handler(e)
  end, ...))
end
return pcall, xpcall
"#;

const SAFE_BASE: &[&str] = &[
    "assert",
    "error",
    "ipairs",
    "next",
    "pairs",
    "select",
    "tonumber",
    "tostring",
    "type",
    "rawequal",
    "rawget",
    "rawlen",
    "getmetatable",
    "_VERSION",
];

pub struct Sandbox {
    lua: Lua,
    env: Table,
    limits: Limits,
}

impl Sandbox {
    pub fn new(limits: Limits) -> mlua::Result<Self> {
        let lua = Lua::new_with(
            StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(limits.memory)?;
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(HOOK_INTERVAL),
            |lua, _debug| {
                if let Ok(Some(mut b)) = lua.try_app_data_mut::<Budget>() {
                    b.used += u64::from(HOOK_INTERVAL);
                    if b.exceeded() {
                        return Err(mlua::Error::external(Interrupt::Timeout));
                    }
                }
                Ok(VmState::Continue)
            },
        )?;
        let env = build_env(&lua)?;
        Ok(Self { lua, env, limits })
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The read-only environment every chunk runs in.
    pub fn env(&self) -> &Table {
        &self.env
    }

    /// Adds a frozen module to the environment (for the stdlib and tests).
    pub fn add_module(&self, name: &str, module: Table) -> mlua::Result<()> {
        let raw: Table = self
            .env
            .metatable()
            .ok_or_else(|| mlua::Error::runtime("env has no metatable"))?
            .get("__index")?;
        raw.set(name, freeze(&self.lua, module)?)
    }

    /// Compiles `source` as a chunk named `name` in the sandbox environment.
    pub fn load(&self, name: &str, source: &str) -> mlua::Result<Function> {
        self.lua
            .load(source)
            .set_name(format!("={name}"))
            .set_environment(self.env.clone())
            .into_function()
    }

    /// Runs `f` with a fresh budget and empty log. Returns the result and
    /// any `print` output.
    pub fn run<R>(
        &self,
        f: impl FnOnce(&Lua) -> mlua::Result<R>,
    ) -> (mlua::Result<R>, Vec<String>) {
        self.lua.set_app_data(Budget {
            used: 0,
            limit: self.limits.instructions,
            deadline: Some(Instant::now() + self.limits.time),
        });
        self.lua.set_app_data(Log::default());
        let result = f(&self.lua);
        self.lua.remove_app_data::<Budget>();
        let log = self.lua.remove_app_data::<Log>().unwrap_or_default().0;
        (result, log)
    }
}

/// Builds the frozen global environment.
fn build_env(lua: &Lua) -> mlua::Result<Table> {
    let globals = lua.globals();
    let raw = lua.create_table()?;
    for name in SAFE_BASE {
        raw.set(*name, globals.get::<LuaValue>(*name)?)?;
    }

    let is_interrupt = lua.create_function(|lua, err: LuaValue| {
        let budget_spent = lua
            .try_app_data_ref::<Budget>()
            .ok()
            .flatten()
            .is_some_and(|b| b.exceeded());
        let hit = match &err {
            LuaValue::Error(e) => interrupt(e).is_some(),
            LuaValue::String(s) => s.as_bytes().as_ref() == b"not enough memory",
            _ => false,
        };
        Ok(budget_spent || hit)
    })?;
    let (pcall, xpcall): (Function, Function) = lua
        .load(PROTECTED_CALLS)
        .set_name("=decel:protected_calls")
        .call((
            globals.get::<Function>("pcall")?,
            globals.get::<Function>("xpcall")?,
            is_interrupt,
            globals.get::<Function>("error")?,
        ))?;
    raw.set("pcall", pcall)?;
    raw.set("xpcall", xpcall)?;

    raw.set(
        "print",
        lua.create_function(|lua, args: MultiValue| {
            let mut parts = Vec::with_capacity(args.len());
            for v in args {
                parts.push(v.to_string()?);
            }
            if let Ok(Some(mut log)) = lua.try_app_data_mut::<Log>()
                && log.0.len() < LOG_LINES
            {
                log.0.push(parts.join("\t"));
            }
            Ok(())
        })?,
    )?;

    // Libraries: copied without unsafe members, then frozen.
    let string = copy_without(lua, &globals.get::<Table>("string")?, &["dump"])?;
    let string = freeze(lua, string)?;
    raw.set("string", string.clone())?;
    let math = copy_without(
        lua,
        &globals.get::<Table>("math")?,
        &["random", "randomseed"],
    )?;
    raw.set("math", freeze(lua, math)?)?;
    let table = copy_without(lua, &globals.get::<Table>("table")?, &[])?;
    raw.set("table", freeze(lua, table)?)?;
    let utf8 = copy_without(lua, &globals.get::<Table>("utf8")?, &[])?;
    raw.set("utf8", freeze(lua, utf8)?)?;

    // Strings index the frozen library, and their metatable is hidden.
    lua.load(
        r#"local s = ...
        local mt = getmetatable("")
        mt.__index = s
        mt.__metatable = false"#,
    )
    .set_name("=decel:string_mt")
    .call::<()>(string)?;

    let env = freeze_with_message(lua, raw.clone(), "globals are read-only (use local)")?;
    raw.set("_G", env.clone())?;
    Ok(env)
}

fn copy_without(lua: &Lua, src: &Table, skip: &[&str]) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for pair in src.pairs::<LuaValue, LuaValue>() {
        let (k, v) = pair?;
        if let LuaValue::String(s) = &k
            && skip.iter().any(|n| s.as_bytes().as_ref() == n.as_bytes())
        {
            continue;
        }
        out.raw_set(k, v)?;
    }
    Ok(out)
}

/// Returns a read-only proxy of `t`.
pub fn freeze(lua: &Lua, t: Table) -> mlua::Result<Table> {
    freeze_with_message(lua, t, "attempt to modify a read-only table")
}

fn freeze_with_message(lua: &Lua, t: Table, message: &'static str) -> mlua::Result<Table> {
    let proxy = lua.create_table()?;
    let mt = lua.create_table()?;
    mt.set("__index", t.clone())?;
    mt.set(
        "__newindex",
        lua.create_function(move |_, _: MultiValue| Err::<(), _>(mlua::Error::runtime(message)))?,
    )?;
    let inner = t.clone();
    mt.set(
        "__pairs",
        lua.create_function(move |lua, _: LuaValue| {
            let next: Function = lua.globals().get("next")?;
            Ok((next, inner.clone(), LuaValue::Nil))
        })?,
    )?;
    let inner = t;
    mt.set(
        "__len",
        lua.create_function(move |_, _: LuaValue| Ok(inner.raw_len()))?,
    )?;
    mt.set("__metatable", "locked")?;
    proxy.set_metatable(Some(mt))?;
    Ok(proxy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb() -> Sandbox {
        Sandbox::new(Limits {
            instructions: 5_000_000,
            time: Duration::from_millis(80),
            memory: 64 * 1024 * 1024,
        })
        .unwrap()
    }

    fn eval(sb: &Sandbox, code: &str) -> Result<String, Fault> {
        let f = sb.load("test", code).map_err(|e| classify(&e, "test"))?;
        let (r, _) = sb.run(|_| f.call::<LuaValue>(()));
        let v = r.map_err(|e| classify(&e, "test"))?;
        Ok(v.to_string().unwrap_or_default())
    }

    fn kind(r: Result<String, Fault>) -> Option<ErrorKind> {
        match r {
            Err(Fault::Error(e)) => Some(e.kind),
            _ => None,
        }
    }

    #[test]
    fn evaluates() {
        assert_eq!(eval(&sb(), "return 1 + 2").unwrap(), "3");
    }

    #[test]
    fn sandbox_is_movable_between_threads() {
        let s = sb();
        let out = std::thread::spawn(move || eval(&s, "return ('ab'):rep(2)").unwrap())
            .join()
            .unwrap();
        assert_eq!(out, "abab");
    }

    #[test]
    fn infinite_loop_times_out_quickly() {
        let s = sb();
        let t = Instant::now();
        assert_eq!(
            kind(eval(&s, "while true do end")),
            Some(ErrorKind::Timeout)
        );
        assert!(
            t.elapsed() < Duration::from_millis(100),
            "{:?}",
            t.elapsed()
        );
        assert_eq!(eval(&s, "return 'ok'").unwrap(), "ok");
    }

    #[test]
    fn pcall_cannot_swallow_interrupts() {
        let s = sb();
        let t = Instant::now();
        assert_eq!(
            kind(eval(
                &s,
                "while true do pcall(function() while true do end end) end"
            )),
            Some(ErrorKind::Timeout)
        );
        assert!(t.elapsed() < Duration::from_millis(100));
        assert_eq!(
            kind(eval(&s, "local ok = pcall(string.rep, 'x', 1e9) return ok")),
            Some(ErrorKind::Memory)
        );
    }

    #[test]
    fn memory_bombs() {
        let s = sb();
        assert_eq!(
            kind(eval(&s, r#"return string.rep("x", 1e10)"#)),
            Some(ErrorKind::Memory)
        );
        assert_eq!(
            kind(eval(&s, r#"return string.rep("x", 1e9)"#)),
            Some(ErrorKind::Memory)
        );
        assert_eq!(eval(&s, "return 'still ok'").unwrap(), "still ok");
    }

    #[test]
    fn pcall_catches_ordinary_errors_with_original_values() {
        let s = sb();
        assert_eq!(
            eval(
                &s,
                "local ok, e = pcall(error, 'boom', 0) return tostring(ok) .. e"
            )
            .unwrap(),
            "falseboom"
        );
        assert_eq!(
            eval(&s, "local ok, e = pcall(error, {code = 7}) return e.code").unwrap(),
            "7"
        );
        assert_eq!(
            eval(
                &s,
                "local ok, e = xpcall(error, function(m) return 'h:' .. m end, 'x', 0) return e"
            )
            .unwrap(),
            "h:x"
        );
    }

    #[test]
    fn escapes_are_blocked() {
        let s = sb();
        for code in [
            "return io.open('x')",
            "return os.time()",
            "return load('return 1')()",
            "return require('os')",
            "return debug.getinfo(1)",
            "return dofile('x')",
            "return loadfile('x')",
            "return collectgarbage('count')",
            "return setmetatable({}, {})",
            "return rawset(_G, 'x', 1)",
            "return string.dump(function() end)",
            "return math.random()",
        ] {
            assert!(eval(&s, code).is_err(), "{code} should fail");
        }
    }

    #[test]
    fn globals_and_libraries_are_read_only() {
        let s = sb();
        assert!(eval(&s, "x = 1").is_err());
        assert!(eval(&s, "string.upper = nil").is_err());
        assert!(eval(&s, "math.pi = 3").is_err());
        assert!(eval(&s, "_G.string = nil").is_err());
        assert_eq!(eval(&s, "return getmetatable('')").unwrap(), "false");
        assert!(eval(&s, "getmetatable('').__index = {}").is_err());
        assert_eq!(eval(&s, "return getmetatable(string)").unwrap(), "locked");
        // State does not leak between evaluations.
        assert_eq!(eval(&s, "return ('x'):upper()").unwrap(), "X");
        assert_eq!(eval(&s, "return math.pi > 3").unwrap(), "true");
        // Iteration over frozen libraries still works.
        assert_eq!(
            eval(
                &s,
                "local n = 0 for _ in pairs(math) do n = n + 1 end return n > 10"
            )
            .unwrap(),
            "true"
        );
    }

    #[test]
    fn print_goes_to_log() {
        let s = sb();
        let f = s.load("p", "print('a', 1) print(nil)").unwrap();
        let (r, log) = s.run(|_| f.call::<()>(()));
        r.unwrap();
        assert_eq!(log, ["a\t1", "nil"]);
    }

    #[test]
    fn errors_carry_member_and_line() {
        let s = sb();
        let f = s.load("value", "local x = 1\nreturn x + nil").unwrap();
        let (r, _) = s.run(|_| f.call::<LuaValue>(()));
        match classify(&r.unwrap_err(), "value") {
            Fault::Error(e) => {
                assert_eq!(e.kind, ErrorKind::Lua);
                assert_eq!(e.member.as_deref(), Some("value"));
                assert_eq!(e.line, Some(2));
                assert!(e.message.contains("arithmetic"), "{}", e.message);
            }
            other => panic!("{other:?}"),
        }
    }
}
