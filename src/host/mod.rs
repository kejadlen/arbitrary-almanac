//! The Fennel host: what turns a `.fnl` workflow into something Almanac can run.
//!
//! A workflow is a file that ends by returning `{: declare : gather : policy}`. Its declaration is
//! a literal, so it is read as data straight from the parsed source, and nothing in the file runs
//! to produce it.

mod capabilities;
mod compile;
mod declaration;
mod fennel;
mod form;
mod gather;
mod registered;
mod sandbox;
mod taint;

pub use capabilities::Capabilities;
pub use declaration::{Agent, Declaration};
pub use gather::{Call, Dispatch};
pub use registered::Registration;

use crate::catalog::{Event, Read};
use crate::{Catalog, Error};
use fennel::{Fennel, Source};
use form::Form;
use sandbox::Sandbox;
use serde_json::Value as Data;

/// A workflow that has been read and checked, but not run.
#[derive(Debug)]
pub struct Workflow {
    source: Source,
    declaration: Declaration,
    capabilities: Capabilities,
    lua: String,
    event: Event,
    /// Every read in the catalog. `gather` reaches only those of the providers it was handed.
    reads: Vec<Read>,
}

impl Workflow {
    /// Reads a workflow from its source. `name` labels errors, usually the file name.
    pub fn load(catalog: &Catalog, name: &str, source: &str) -> Result<Self, Error> {
        let source = Source::new(name, source);
        let fennel = Fennel::new()?;
        let forms = fennel.parse(&source)?;
        check_exports(&source, forms.last())?;
        let declaration = read_declaration(&source, &forms)?;
        let event = catalog
            .event(&declaration.on)
            .cloned()
            .ok_or_else(|| Error::Event {
                on: declaration.on.clone(),
                src: source.named(),
            })?;
        let capabilities = capabilities::read(catalog, &declaration, &source, &forms)?;
        let lua = compile::compile(&fennel, &source, &forms)?;
        Ok(Self {
            source,
            declaration,
            capabilities,
            lua,
            event,
            reads: catalog.reads.clone(),
        })
    }

    /// Runs `gather` on `event` in a fresh sandbox, handing it only the providers and agents it
    /// asked for, and returns its facts.
    pub async fn gather(&self, event: &Data, dispatch: &mut impl Dispatch) -> Result<Data, Error> {
        let sandbox = Sandbox::new()?;
        // The module's own locals run here too, so their failures are the run's.
        let exports = sandbox
            .exports(self.source.name(), &self.lua)
            .map_err(|error| gather::failed(&self.source, error))?;
        let capabilities = &self.capabilities;
        let granted: Vec<&str> = capabilities
            .providers
            .iter()
            .chain(&capabilities.agents)
            .map(String::as_str)
            .collect();
        gather::run(&sandbox, &self.source, &exports, &granted, event, dispatch).await
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
mod gather_tests;
#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
