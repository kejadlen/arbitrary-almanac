//! A Lua VM holding the vendored Fennel compiler, used here only to read source as data.
//!
//! The VM has no `io`, `os`, or `debug`. Nothing a workflow wrote runs in it: the parser turns
//! source into tables, and they cross into Rust as [`Form`]s.

use super::form::{self, Form};
use crate::Error;
use miette::{NamedSource, SourceSpan};
use mlua::{Function, Lua, LuaOptions, StdLib, Table};

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
        lua.set_named_registry_value(form::FENNEL, &module)?;
        Ok(Self { lua, module })
    }

    /// Every top-level form in `source`.
    pub fn parse(&self, source: &Source) -> Result<Vec<Form>, Error> {
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
}
