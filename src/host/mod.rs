//! The Fennel host: what turns a `.fnl` workflow into something Almanac can run.
//!
//! A workflow is a file that ends by returning `{: declare : gather : policy}`. Its declaration is
//! a literal, so it is read as data straight from the parsed source, and nothing in the file runs
//! to produce it.

mod declaration;
mod fennel;

pub use declaration::{Agent, Declaration, May};

use crate::Error;
use fennel::{Fennel, Source};
#[cfg(test)]
use miette::SourceSpan;
use mlua::{LuaSerdeExt as _, Value};

/// A workflow that has been read and checked, but not run.
#[derive(Debug)]
pub struct Workflow {
    declaration: Declaration,
}

impl Workflow {
    /// Reads a workflow from its source. `name` labels errors, usually the file name.
    pub fn load(name: &str, source: &str) -> Result<Self, Error> {
        let fennel = Fennel::new()?;
        let source = Source::new(name, source);
        let forms = fennel.parse(&source)?;
        check_exports(&fennel, &source, forms.last())?;
        let declaration = read_declaration(&fennel, &source, &forms)?;
        Ok(Self { declaration })
    }

    pub fn declaration(&self) -> &Declaration {
        &self.declaration
    }
}

const EXPORTS: [&str; 3] = ["declare", "gather", "policy"];

/// The last form must be `{: declare : gather : policy}` and nothing else, so what a workflow
/// hands the host is the same in every file.
fn check_exports(fennel: &Fennel, source: &Source, last: Option<&Value>) -> Result<(), Error> {
    let refused = || Error::Exports {
        src: source.named(),
        at: last.and_then(|form| fennel.span(form)),
    };
    let Some(last @ Value::Table(table)) = last else {
        return Err(refused());
    };
    if fennel.is_list(last)? || table.pairs::<Value, Value>().count() != EXPORTS.len() {
        return Err(refused());
    }
    for export in EXPORTS {
        let value: Value = table.get(export)?;
        if fennel.symbol(&value)?.as_deref() != Some(export) {
            return Err(refused());
        }
    }
    Ok(())
}

/// Finds the one top-level `(local declare ...)` and reads it as data.
fn read_declaration(
    fennel: &Fennel,
    source: &Source,
    forms: &[Value],
) -> Result<Declaration, Error> {
    let mut found = Vec::new();
    for form in forms {
        if let Some(value) = local_named(fennel, form, "declare")? {
            found.push(value);
        }
    }
    let [value] = found.as_slice() else {
        return Err(Error::Undeclared {
            src: source.named(),
        });
    };
    if let Some(bad) = fennel.first_non_literal(value)? {
        return Err(Error::NotLiteral {
            src: source.named(),
            at: fennel.span(&bad),
        });
    }
    fennel
        .lua()
        .from_value(value.clone())
        .map_err(|e| Error::Declaration {
            message: e.to_string(),
            src: source.named(),
            at: fennel.span(value),
        })
}

/// The value of `(local <name> value)`, if `form` is that.
fn local_named(fennel: &Fennel, form: &Value, name: &str) -> Result<Option<Value>, Error> {
    let Value::Table(list) = form else {
        return Ok(None);
    };
    if !fennel.is_list(form)? {
        return Ok(None);
    }
    let head: Value = list.get(1)?;
    let target: Value = list.get(2)?;
    if fennel.symbol(&head)?.as_deref() == Some("local")
        && fennel.symbol(&target)?.as_deref() == Some(name)
    {
        return Ok(Some(list.get(3)?));
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
