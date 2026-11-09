//! Running `gather`: the half of a workflow that reads.
//!
//! `gather` runs as a coroutine in a fresh [`Sandbox`], handed only the capabilities it
//! destructured. Each is an opaque handle whose methods yield the call back to Rust, which
//! performs it through a [`Dispatch`] and resumes the script with the answer. The script never
//! holds a client, a socket, or the run's context, only what Rust hands back.

use super::fennel::Source;
use super::sandbox::Sandbox;
use super::taint;
use crate::Error;
use mlua::prelude::LuaThreadStatus as ThreadStatus;
use mlua::{
    IntoLuaMulti as _, LuaSerdeExt as _, MetaMethod, MultiValue, Table, UserData, UserDataMethods,
    Value, Variadic,
};
use serde_json::Value as Data;
use std::future::Future;

/// A call a workflow made on one of its capabilities, such as `(github.pull repo 3)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub capability: String,
    pub method: String,
    pub args: Vec<Data>,
}

/// What performs a workflow's calls. In a run this goes through Almanac's `Cx`, so each call is
/// recorded and a replay answers from the record.
pub trait Dispatch {
    fn call(&mut self, call: Call) -> impl Future<Output = Result<Data, String>> + Send;
}

/// A capability as a script holds it: a name, and methods that yield.
struct Capability(String);

impl UserData for Capability {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Index, |lua, this, method: String| {
            let capability = this.0.clone();
            lua.create_async_function(move |lua, args: MultiValue| {
                let call = (capability.clone(), method.clone(), args);
                async move { lua.yield_with::<Value>(call).await }
            })
        });
    }
}

pub async fn run(
    sandbox: &Sandbox,
    source: &Source,
    exports: &Table,
    granted: &[&str],
    event: &Data,
    dispatch: &mut impl Dispatch,
) -> Result<Data, Error> {
    let lua = sandbox.lua();
    let thread = sandbox.thread(exports.get("gather")?)?;
    let capabilities = lua.create_table()?;
    for name in granted {
        capabilities.set(*name, Capability(name.to_string()))?;
    }
    let mut input = (capabilities, taint::into_lua(lua, event)?).into_lua_multi(lua)?;
    let failed = |error| failed(source, error);
    loop {
        let output: MultiValue = thread.resume(input).map_err(failed)?;
        if thread.status() != ThreadStatus::Resumable {
            let facts = output.into_iter().next().unwrap_or(Value::Nil);
            return lua.from_value(facts).map_err(failed);
        }
        // A script has no `coroutine`, so every yield is a capability's.
        let (capability, method, args): (String, String, Variadic<Value>) =
            lua.unpack_multi(output)?;
        let args = args
            .into_iter()
            .map(|arg| lua.from_value(arg))
            .collect::<mlua::Result<_>>()
            .map_err(failed)?;
        let call = Call {
            capability,
            method,
            args,
        };
        let answer = dispatch.call(call).await.map_err(|message| Error::Run {
            function: "gather",
            message,
            src: source.named(),
            at: None,
        })?;
        input = taint::into_lua(lua, &answer)?.into_lua_multi(lua)?;
    }
}

pub fn failed(source: &Source, error: mlua::Error) -> Error {
    let (message, at) = source.located(&error);
    Error::Run {
        function: "gather",
        message,
        src: source.named(),
        at,
    }
}
