//! The Fennel host: what turns a `.fnl` workflow into something Almanac can run.
//!
//! A workflow is a file that ends by returning `{: declare : gather : policy}`. Its declaration is
//! a literal, so it is read as data straight from the parsed source, and nothing in the file runs
//! to produce it.

mod capabilities;
mod declaration;
mod fennel;
mod form;

pub use capabilities::Capabilities;
pub use declaration::{Agent, Declaration};

use crate::{Catalog, Error};
use fennel::{Fennel, Source};
use form::Form;

/// A workflow that has been read and checked, but not run.
#[derive(Debug)]
pub struct Workflow {
    declaration: Declaration,
    capabilities: Capabilities,
}

impl Workflow {
    /// Reads a workflow from its source. `name` labels errors, usually the file name.
    pub fn load(catalog: &Catalog, name: &str, source: &str) -> Result<Self, Error> {
        let source = Source::new(name, source);
        let forms = Fennel::new()?.parse(&source)?;
        check_exports(&source, forms.last())?;
        let declaration = read_declaration(&source, &forms)?;
        let capabilities = capabilities::read(catalog, &declaration, &source, &forms)?;
        Ok(Self {
            declaration,
            capabilities,
        })
    }

    pub fn declaration(&self) -> &Declaration {
        &self.declaration
    }

    /// What `gather` and `policy` may use, from what they destructure.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }
}

const EXPORTS: [&str; 3] = ["declare", "gather", "policy"];

/// The last form must be `{: declare : gather : policy}` and nothing else, so what a workflow
/// hands the host is the same in every file.
fn check_exports(source: &Source, last: Option<&Form>) -> Result<(), Error> {
    let exports = |entries: &[(Form, Form)]| {
        entries.len() == EXPORTS.len()
            && EXPORTS.iter().all(|export| {
                entries.iter().any(|(key, value)| {
                    *key == Form::Str(export.to_string()) && value.symbol() == Some(export)
                })
            })
    };
    match last {
        Some(Form::Table(entries, _)) if exports(entries) => Ok(()),
        _ => Err(Error::Exports {
            src: source.named(),
            at: last.and_then(Form::span),
        }),
    }
}

/// Finds the one top-level `(local declare ...)` and reads it as data.
fn read_declaration(source: &Source, forms: &[Form]) -> Result<Declaration, Error> {
    let found: Vec<&Form> = forms
        .iter()
        .filter_map(|f| local_named(f, "declare"))
        .collect();
    let [form] = found.as_slice() else {
        return Err(Error::Undeclared {
            src: source.named(),
        });
    };
    let data = form.to_data().map_err(|bad| Error::NotLiteral {
        src: source.named(),
        at: bad.span(),
    })?;
    serde_json::from_value(data).map_err(|e| Error::Declaration {
        message: e.to_string(),
        src: source.named(),
        at: form.span(),
    })
}

/// The value of `(local <name> value)`, if `form` is that.
fn local_named<'a>(form: &'a Form, name: &str) -> Option<&'a Form> {
    match form {
        Form::List(items, _) => match items.as_slice() {
            [head, target, value, ..]
                if head.symbol() == Some("local") && target.symbol() == Some(name) =>
            {
                Some(value)
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests;
