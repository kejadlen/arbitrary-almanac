//! Turning a checked workflow into the Lua a run executes.
//!
//! Before Fennel sees the source, Rust refuses what would run code at compile time or on every
//! load: macros and their kin anywhere, and anything at the top level but locals and functions.

use super::fennel::{Fennel, Source};
use super::form::Form;
use crate::Error;

/// The globals a workflow may name. Each is pure, and [`super::sandbox`] hands a run exactly
/// these. `_G` is here because Fennel's `case` compiles to `_G.type`.
pub const GLOBALS: [&str; 11] = [
    "_G", "assert", "error", "ipairs", "math", "pairs", "select", "string", "table", "tonumber",
    "type",
];

/// Forms that run code inside the compiler, or splice in code the reader never saw.
const COMPILE_TIME: [&str; 9] = [
    "eval-compiler",
    "import-macros",
    "include",
    "lua",
    "macro",
    "macrodebug",
    "macros",
    "require-macros",
    "quote",
];

pub fn compile(fennel: &Fennel, source: &Source, forms: &[Form]) -> Result<String, Error> {
    let body = forms.split_last().map_or(&[][..], |(_, body)| body);
    for form in body {
        let head = match form {
            Form::List(items, _) => items.first().and_then(Form::symbol),
            _ => None,
        };
        if !matches!(head, Some("local" | "fn")) {
            return Err(Error::TopLevel {
                src: source.named(),
                at: form.span(),
            });
        }
    }
    for form in forms {
        refuse_compile_time(source, form)?;
    }
    fennel.compile(source, &GLOBALS)
}

fn refuse_compile_time(source: &Source, form: &Form) -> Result<(), Error> {
    let inner: Vec<&Form> = match form {
        Form::List(items, span) => {
            if let Some(head) = items.first().and_then(Form::symbol)
                && COMPILE_TIME.contains(&head)
            {
                return Err(Error::CompileTime {
                    form: head.to_string(),
                    src: source.named(),
                    at: *span,
                });
            }
            items.iter().collect()
        }
        Form::Sequence(items, _) => items.iter().collect(),
        Form::Table(entries, _) => entries.iter().flat_map(|(k, v)| [k, v]).collect(),
        _ => vec![],
    };
    for form in inner {
        refuse_compile_time(source, form)?;
    }
    Ok(())
}
