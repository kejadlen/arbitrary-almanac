//! A Lua VM holding the vendored Fennel compiler, used here only to read source as data.
//!
//! The VM has no `io`, `os`, or `debug`. Nothing a workflow wrote runs in it: the parser turns
//! source into tables, and they cross into Rust as [`Form`]s.

use super::form::{self, Form};
use crate::Error;
use miette::{NamedSource, SourceSpan};
use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value};
use std::sync::Arc;

const FENNEL: &str = include_str!("../../vendor/fennel.lua");

/// A workflow's text and the name its errors carry.
#[derive(Debug)]
pub struct Source {
    name: String,
    text: String,
    /// Shared by every error about this source, so each one carries only a pointer.
    named: Arc<NamedSource<String>>,
}

impl Source {
    pub fn new(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            text: text.to_string(),
            named: Arc::new(NamedSource::new(name, text.to_string())),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn named(&self) -> Arc<NamedSource<String>> {
        Arc::clone(&self.named)
    }

    /// The first line of a Fennel or Lua error, and where in this source it points.
    ///
    /// Fennel and Lua's `error` start the message with its position. An error raised from Rust
    /// has none of its own, so it takes the traceback's first frame in this source.
    pub(super) fn located(&self, error: &mlua::Error) -> (String, Option<SourceSpan>) {
        let message = match root(error) {
            mlua::Error::RuntimeError(message) => message.clone(),
            other => other.to_string(),
        };
        let first = message.lines().next().unwrap_or_default();
        if let Some((_, at)) = self.position(first) {
            return (first.to_string(), Some(at));
        }
        let frames = match error {
            mlua::Error::CallbackError { traceback, .. } => traceback.lines(),
            _ => "".lines(),
        };
        let frame = frames
            .map(str::trim_start)
            .find_map(|frame| self.position(frame).map(|(len, at)| (&frame[..len], at)));
        match frame {
            Some((position, at)) => (format!("{position} {first}"), Some(at)),
            None => (first.to_string(), None),
        }
    }

    /// A position such as `w.fnl:3:7:` or `w.fnl:3:` at the start of `text`: how long it is, and
    /// where in this source it points. Lua's own errors have no column.
    fn position(&self, text: &str) -> Option<(usize, SourceSpan)> {
        let rest = text.strip_prefix(&self.name)?.strip_prefix(':')?;
        let (line, rest) = number(rest)?;
        let (col, rest) = number(rest).unwrap_or((0, rest));
        let start: usize = self
            .text
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum();
        let offset = start.saturating_add(col).min(self.text.len());
        Some((
            text.len().saturating_sub(rest.len()),
            SourceSpan::from((offset, 0)),
        ))
    }
}

/// The error under any callback wrapping, which is the one that says what went wrong.
fn root(error: &mlua::Error) -> &mlua::Error {
    match error {
        mlua::Error::CallbackError { cause, .. } => root(cause),
        other => other,
    }
}

/// A number and a colon at the start of `text`, such as the `3:` of `3:7: Parse error`.
fn number(text: &str) -> Option<(usize, &str)> {
    let (digits, rest) = text.split_once(':')?;
    Some((digits.parse().ok()?, rest))
}

/// A VM with the Fennel module in its registry, where [`Form`]'s conversion also finds it.
pub struct Fennel {
    lua: Lua,
}

impl Fennel {
    pub fn new() -> Result<Self, Error> {
        let libs = StdLib::PACKAGE | StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        let module: Table = lua.load(FENNEL).set_name("fennel.lua").eval()?;
        lua.set_named_registry_value(form::FENNEL, &module)?;
        Ok(Self { lua })
    }

    /// Every top-level form in `source`, read by driving Fennel's `parser` iterator from Rust.
    pub fn parse(&self, source: &Source) -> Result<Vec<Form>, Error> {
        let fennel: Table = self.lua.named_registry_value(form::FENNEL)?;
        let parser: Function = fennel.get("parser")?;
        let read = || -> mlua::Result<Vec<Form>> {
            let (next, state): (Function, Value) =
                parser.call((source.text.as_str(), source.name.as_str()))?;
            let mut forms = Vec::new();
            while let (_, Some(form)) = next.call::<(Value, Option<Form>)>(&state)? {
                forms.push(form);
            }
            Ok(forms)
        };
        read().map_err(|e| {
            let (message, at) = source.located(&e);
            Error::Parse {
                message,
                src: source.named(),
                at,
            }
        })
    }

    /// Compiles `source` to Lua. A name that is neither a local nor in `globals` is a compile
    /// error, so a workflow cannot even mention what a run will not hand it.
    pub fn compile(&self, source: &Source, globals: &[&str]) -> Result<String, Error> {
        let fennel: Table = self.lua.named_registry_value(form::FENNEL)?;
        let compile: Function = fennel.get("compileString")?;
        let options = self.lua.create_table()?;
        options.set("filename", source.name.as_str())?;
        // Keep Lua's line numbers on the Fennel lines they came from, so run errors point there.
        options.set("correlate", true)?;
        let allowed = self.lua.create_sequence_from(globals.iter().copied())?;
        options.set("allowedGlobals", allowed)?;
        compile.call((source.text.as_str(), options)).map_err(|e| {
            let (message, at) = source.located(&e);
            Error::Compile {
                message,
                src: source.named(),
                at,
            }
        })
    }
}
