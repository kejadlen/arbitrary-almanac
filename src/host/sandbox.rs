//! The VM a run executes a workflow's Lua in.
//!
//! It is new for every run and sees only [`super::compile::GLOBALS`]. `pairs` walks keys in a
//! fixed order, `math` has no `random`, and a run is stopped after a fixed number of
//! instructions, so the same inputs make the same run.

use super::compile::GLOBALS;
use crate::Error;
use mlua::prelude::LuaChunkMode as ChunkMode;
use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use std::cmp::Ordering;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering as Memory};

/// How many instructions a run may execute, in thousands.
const BUDGET: u32 = 10_000;

pub struct Sandbox {
    lua: Lua,
    env: Table,
}

impl Sandbox {
    pub fn new() -> Result<Self, Error> {
        let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        let globals = lua.globals();
        let env = lua.create_table()?;
        for name in GLOBALS {
            env.set(name, globals.get::<Value>(name)?)?;
        }
        env.set("_G", &env)?;
        env.set("pairs", lua.create_function(sorted_pairs)?)?;
        let math: Table = globals.get("math")?;
        let pure_math = lua.create_table()?;
        for pair in math.pairs::<String, Value>() {
            let (name, value) = pair?;
            if !matches!(name.as_str(), "random" | "randomseed") {
                pure_math.set(name, value)?;
            }
        }
        env.set("math", pure_math)?;
        // `("").dump` reaches the real string table through the string metatable.
        globals.get::<Table>("string")?.set("dump", Value::Nil)?;
        Ok(Self { lua, env })
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Runs compiled source and returns what it exports. Only text is accepted, never bytecode.
    pub fn exports(&self, name: &str, lua: &str) -> mlua::Result<Table> {
        self.lua
            .load(lua)
            .set_name(format!("@{name}"))
            .set_mode(ChunkMode::Text)
            .set_environment(self.env.clone())
            .eval()
    }

    /// A coroutine for `function` that stops once it has spent its instruction budget.
    pub fn thread(&self, function: Function) -> Result<mlua::Thread, Error> {
        let thread = self.lua.create_thread(function)?;
        let left = AtomicU32::new(BUDGET);
        let every = HookTriggers::new().every_nth_instruction(1000);
        let hooked = thread.set_hook(every, move |_, _| {
            // The hook runs only on this thread's own coroutine, so a load and a store do not
            // race. `fetch_update` is deprecated on newer toolchains, and `try_update` is not on
            // older ones.
            match left.load(Memory::SeqCst).checked_sub(1) {
                Some(n) => {
                    left.store(n, Memory::SeqCst);
                    Ok(VmState::Continue)
                }
                None => Err(mlua::Error::runtime("ran past its instruction budget")),
            }
        });
        hooked?;
        Ok(thread)
    }
}

/// `pairs` in a fixed order: numbers ascending, then strings ascending. Lua's own order depends
/// on a per-VM hash seed, so two runs could walk the same table differently.
fn sorted_pairs(lua: &Lua, table: Table) -> mlua::Result<(Function, Value, Value)> {
    let mut entries = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let rank = match &key {
            // Integers too far apart for an f64 to tell them apart fall back on themselves.
            Value::Integer(n) => (0, *n as f64, *n, vec![]),
            Value::Number(n) => (0, *n, *n as i64, vec![]),
            Value::String(s) => (1, 0.0, 0, s.as_bytes().to_vec()),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "pairs walks only tables keyed by numbers and strings, not a {}",
                    other.type_name()
                )));
            }
        };
        entries.push((rank, key, value));
    }
    entries.sort_by(|(a, ..), (b, ..)| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let entries = Mutex::new(entries.into_iter().map(|(_, key, value)| (key, value)));
    let next = lua.create_function(move |_, ()| {
        let next = entries.lock().ok().and_then(|mut e| e.next());
        Ok(next.unwrap_or((Value::Nil, Value::Nil)))
    })?;
    Ok((next, Value::Nil, Value::Nil))
}
