//! Fennel source as Rust data. The parser's tables cross into Rust once, through `FromLua`, and
//! everything after that is a pattern match.

use miette::SourceSpan;
use mlua::{FromLua, Function, Lua, Table, Value};
use serde_json::{Map, Value as Data};

/// Where the host keeps the Fennel module, so a conversion can ask what a table is.
pub const FENNEL: &str = "arbitrary_almanac.fennel";

/// One parsed form. Strings, numbers, and booleans carry no position, because the parser
/// records none for them.
#[derive(Clone, Debug, PartialEq)]
pub enum Form {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Str(String),
    Symbol(String, Option<SourceSpan>),
    List(Vec<Form>, Option<SourceSpan>),
    Sequence(Vec<Form>, Option<SourceSpan>),
    Table(Vec<(Form, Form)>, Option<SourceSpan>),
}

impl Form {
    pub fn span(&self) -> Option<SourceSpan> {
        match self {
            Form::Symbol(_, span) | Form::List(_, span) | Form::Sequence(_, span) => *span,
            Form::Table(_, span) => *span,
            Form::Bool(_) | Form::Integer(_) | Form::Number(_) | Form::Str(_) => None,
        }
    }

    pub fn symbol(&self) -> Option<&str> {
        match self {
            Form::Symbol(name, _) => Some(name),
            _ => None,
        }
    }

    /// This form as plain data, or the first part of it that is not: a symbol, which would name
    /// something, or a list, which would call something.
    pub fn to_data(&self) -> Result<Data, &Form> {
        match self {
            Form::Bool(b) => Ok(Data::from(*b)),
            Form::Integer(n) => Ok(Data::from(*n)),
            Form::Number(n) => Ok(Data::from(*n)),
            Form::Str(s) => Ok(Data::from(s.as_str())),
            Form::Symbol(..) | Form::List(..) => Err(self),
            Form::Sequence(items, _) => items.iter().map(Form::to_data).collect(),
            Form::Table(entries, _) => {
                let mut map = Map::new();
                for (key, value) in entries {
                    let key = match key {
                        Form::Str(s) => s.clone(),
                        other => other.to_data()?.to_string(),
                    };
                    map.insert(key, value.to_data()?);
                }
                Ok(Data::Object(map))
            }
        }
    }
}

impl FromLua for Form {
    fn from_lua(value: Value, lua: &Lua) -> mlua::Result<Self> {
        let table = match value {
            Value::Boolean(b) => return Ok(Form::Bool(b)),
            Value::Integer(n) => return Ok(Form::Integer(n)),
            Value::Number(n) => return Ok(Form::Number(n)),
            Value::String(s) => return Ok(Form::Str(s.to_str()?.to_string())),
            Value::Table(table) => table,
            other => {
                return Err(mlua::Error::FromLuaConversionError {
                    from: other.type_name(),
                    to: "Form".to_string(),
                    message: None,
                });
            }
        };
        let fennel: Table = lua.named_registry_value(FENNEL)?;
        let is = |predicate: &str| -> mlua::Result<bool> {
            let predicate: Function = fennel.get(predicate)?;
            let answer: Value = predicate.call(&table)?;
            Ok(!matches!(answer, Value::Nil | Value::Boolean(false)))
        };
        let span = span(&fennel, &table);
        if is("sym?")? || is("varg?")? {
            return Ok(Form::Symbol(table.get(1)?, span));
        }
        if is("list?")? {
            return Ok(Form::List(
                table.sequence_values().collect::<Result<_, _>>()?,
                span,
            ));
        }
        if is("sequence?")? {
            return Ok(Form::Sequence(
                table.sequence_values().collect::<Result<_, _>>()?,
                span,
            ));
        }
        Ok(Form::Table(table.pairs().collect::<Result<_, _>>()?, span))
    }
}

/// Where a parsed table came from, when the parser recorded it.
pub fn span(fennel: &Table, table: &Table) -> Option<SourceSpan> {
    let source: Function = fennel.get("ast-source").ok()?;
    let source: Table = source.call(table).ok()?;
    let start: usize = source.get("bytestart").ok()?;
    let end: usize = source.get("byteend").ok()?;
    let len = end.checked_add(1)?.checked_sub(start)?;
    Some(SourceSpan::from((start.checked_sub(1)?, len)))
}
