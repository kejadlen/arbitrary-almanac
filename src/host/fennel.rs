//! A Lua VM holding the vendored Fennel compiler, used here only to read source as data.
//!
//! The VM has no `io`, `os`, or `debug`. Nothing a workflow wrote runs in it: the parser turns
//! source into tables, and the host walks those tables from Rust.

use crate::Error;
use miette::{NamedSource, SourceSpan};
use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value};

const FENNEL: &str = include_str!("../../vendor/fennel.lua");

/// Collects every top-level form of a source string into one sequence.
const PARSE: &str = r"
local fennel, source, name = ...
local forms = {}
for _, form in fennel.parser(source, name) do
  table.insert(forms, form)
end
return forms
";

/// A workflow's text and the name its errors carry.
pub struct Source {
    name: String,
    text: String,
}

impl Source {
    pub fn new(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            text: text.to_string(),
        }
    }

    pub fn named(&self) -> NamedSource<String> {
        NamedSource::new(&self.name, self.text.clone())
    }

    /// Where a Fennel error message such as `w.fnl:3:7: Parse error: ...` points.
    pub(super) fn offset_of(&self, message: &str) -> Option<SourceSpan> {
        let rest = message.strip_prefix(&self.name)?.strip_prefix(':')?;
        let mut parts = rest.splitn(3, ':');
        let line: usize = parts.next()?.parse().ok()?;
        let col: usize = parts.next()?.parse().ok()?;
        let start: usize = self
            .text
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum();
        let offset = start.saturating_add(col).min(self.text.len());
        Some(SourceSpan::from((offset, 0)))
    }
}

pub struct Fennel {
    lua: Lua,
    module: Table,
}

impl Fennel {
    pub fn new() -> Result<Self, Error> {
        let libs = StdLib::PACKAGE | StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        let module: Table = lua.load(FENNEL).set_name("fennel.lua").eval()?;
        Ok(Self { lua, module })
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Every top-level form in `source`, as the parser's tables.
    pub fn parse(&self, source: &Source) -> Result<Vec<Value>, Error> {
        let parse: Function = self.lua.load(PARSE).set_name("parse").into_function()?;
        parse
            .call((&self.module, source.text.as_str(), source.name.as_str()))
            .map_err(|e| {
                let message = e.to_string();
                let message = message.strip_prefix("runtime error: ").unwrap_or(&message);
                let first = message.lines().next().unwrap_or_default().to_string();
                Error::Parse {
                    at: source.offset_of(&first),
                    message: first,
                    src: source.named(),
                }
            })
    }

    fn call(&self, function: &str, value: &Value) -> Result<Value, Error> {
        let function: Function = self.module.get(function)?;
        Ok(function.call(value)?)
    }

    pub fn is_list(&self, value: &Value) -> Result<bool, Error> {
        Ok(truthy(&self.call("list?", value)?))
    }

    /// The name of a symbol, or `None` for anything else.
    pub fn symbol(&self, value: &Value) -> Result<Option<String>, Error> {
        match self.call("sym?", value)? {
            Value::Table(symbol) => Ok(Some(symbol.get(1)?)),
            _ => Ok(None),
        }
    }

    /// The first part of `value` that is not plain data: a symbol, which would name something,
    /// or a list, which would call something.
    pub fn first_non_literal(&self, value: &Value) -> Result<Option<Value>, Error> {
        let Value::Table(table) = value else {
            return Ok(None);
        };
        if self.symbol(value)?.is_some() || self.is_list(value)? {
            return Ok(Some(value.clone()));
        }
        for pair in table.pairs::<Value, Value>() {
            let (key, inner) = pair?;
            for part in [key, inner] {
                if let Some(bad) = self.first_non_literal(&part)? {
                    return Ok(Some(bad));
                }
            }
        }
        Ok(None)
    }

    /// Where a parsed form came from, when the parser recorded it.
    pub fn span(&self, value: &Value) -> Option<SourceSpan> {
        let source = self.call("ast-source", value).ok()?;
        let source = source.as_table()?;
        let start: usize = source.get("bytestart").ok()?;
        let end: usize = source.get("byteend").ok()?;
        let len = end.checked_add(1)?.checked_sub(start)?;
        Some(SourceSpan::from((start.checked_sub(1)?, len)))
    }
}

fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Nil | Value::Boolean(false))
}
