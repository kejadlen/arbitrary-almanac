//! Untrusted text as a script holds it.
//!
//! Almanac serializes an `Untrusted<T>` as `{"$untrusted": T}`. Crossing into a script, that
//! object becomes an opaque [`Tainted`] handle instead of a table or a string, so a workflow can
//! carry hostile text into its facts but cannot read, compare, concatenate, or format it. Crossing
//! back, the handle serializes as the same object, so the text stays `Untrusted` on the Rust side.

use mlua::{Lua, LuaSerdeExt as _, MetaMethod, UserData, UserDataMethods, Value};
use serde::Serialize;
use serde_json::Value as Data;

const KEY: &str = "$untrusted";

#[derive(Serialize)]
struct Tainted {
    #[serde(rename = "$untrusted")]
    inner: Data,
}

impl UserData for Tainted {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Lua would otherwise print a type name and an address, which differs between runs.
        methods.add_meta_method(MetaMethod::ToString, |_, _, ()| {
            Err::<(), _>(mlua::Error::runtime(
                "untrusted text cannot become a string",
            ))
        });
    }
}

/// `data` as a script sees it, with every `{"$untrusted": ...}` an opaque handle.
pub fn into_lua(lua: &Lua, data: &Data) -> mlua::Result<Value> {
    match data {
        Data::Object(object) => match object.get(KEY) {
            Some(inner) if object.len() == 1 => {
                let inner = inner.clone();
                Ok(Value::UserData(lua.create_ser_userdata(Tainted { inner })?))
            }
            _ => {
                let table = lua.create_table()?;
                for (key, value) in object {
                    table.set(key.as_str(), into_lua(lua, value)?)?;
                }
                Ok(Value::Table(table))
            }
        },
        Data::Array(items) => {
            let table = lua.create_table()?;
            for item in items {
                table.push(into_lua(lua, item)?)?;
            }
            table.set_metatable(Some(lua.array_metatable()))?;
            Ok(Value::Table(table))
        }
        scalar => lua.to_value(scalar),
    }
}
