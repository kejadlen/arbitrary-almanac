//! What `gather` and `policy` may use, read from what they destructure.
//!
//! `(fn gather [{: github : fixer} event] ...)` asks for the `github` provider and the `fixer`
//! agent, and `(fn policy [facts {: attest}] ...)` for the `attest` effect. A run hands each
//! function exactly those, so a name it did not destructure is a name it cannot reach.

use super::declaration::{Declaration, effect_kind};
use super::fennel::Source;
use super::form::Form;
use crate::{Catalog, Error};
use almanac::effect::EffectKind;

/// The manifest's half that comes from the functions rather than the declaration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Capabilities {
    pub providers: Vec<String>,
    pub agents: Vec<String>,
    pub effects: Vec<EffectKind>,
}

pub fn read(
    catalog: &Catalog,
    declaration: &Declaration,
    source: &Source,
    forms: &[Form],
) -> Result<Capabilities, Error> {
    let mut capabilities = Capabilities::default();

    let gather = &function(source, forms, "gather")?;
    let [Form::Table(asked, _), Form::Symbol(..)] = gather.params else {
        return Err(parameters(source, gather, "[{: provider : agent} event]"));
    };
    for (name, at) in names(source, gather, asked)? {
        if catalog.providers.contains(&name.as_str()) {
            capabilities.providers.push(name);
        } else if declaration.agents.contains_key(&name) {
            capabilities.agents.push(name);
        } else {
            return Err(refused(
                source,
                gather,
                name,
                at,
                "no provider or agent has that name",
            ));
        }
    }

    let policy = &function(source, forms, "policy")?;
    let mut effects = Vec::new();
    let [Form::Symbol(..), Form::Table(asked, _)] = policy.params else {
        return Err(parameters(source, policy, "[facts {: effect}]"));
    };
    for (name, at) in names(source, policy, asked)? {
        match effect_kind(&name) {
            Some(kind) if declaration.may.contains(&kind) => effects.push(kind),
            Some(_) => return Err(refused(source, policy, name, at, "it is not in :may")),
            None => {
                return Err(refused(
                    source,
                    policy,
                    name,
                    at,
                    "Almanac has no such effect",
                ));
            }
        }
    }

    capabilities.providers.sort();
    capabilities.agents.sort();
    // In the order `:may` lists them, since Almanac's effects have no order of their own.
    capabilities.effects = declaration
        .may
        .iter()
        .copied()
        .filter(|kind| effects.contains(kind))
        .collect();
    Ok(capabilities)
}

/// A top-level `(fn <name> [params] ...)`.
struct Function<'a> {
    name: &'static str,
    params: &'a [Form],
    span: Option<miette::SourceSpan>,
}

fn function<'a>(
    source: &Source,
    forms: &'a [Form],
    name: &'static str,
) -> Result<Function<'a>, Error> {
    let found: Vec<Function> = forms
        .iter()
        .filter_map(|form| match form {
            Form::List(items, span) => match items.as_slice() {
                [head, target, Form::Sequence(params, _), ..]
                    if head.symbol() == Some("fn") && target.symbol() == Some(name) =>
                {
                    Some(Function {
                        name,
                        params,
                        span: *span,
                    })
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    match <[Function; 1]>::try_from(found) {
        Ok([function]) => Ok(function),
        Err(_) => Err(Error::Undefined {
            function: name,
            src: source.named(),
        }),
    }
}

/// Each `name binding` pair of a destructuring table, with where its binding is.
fn names(
    source: &Source,
    function: &Function,
    entries: &[(Form, Form)],
) -> Result<Vec<(String, Option<miette::SourceSpan>)>, Error> {
    let mut names = Vec::new();
    for (key, binding) in entries {
        match (key, binding) {
            (Form::Str(name), Form::Symbol(_, at)) => names.push((name.clone(), *at)),
            _ => {
                return Err(Error::Parameters {
                    function: function.name,
                    expected: "a plain `: name` for each capability",
                    src: source.named(),
                    at: binding.span().or(function.span),
                });
            }
        }
    }
    Ok(names)
}

fn parameters(source: &Source, function: &Function, expected: &'static str) -> Error {
    Error::Parameters {
        function: function.name,
        expected,
        src: source.named(),
        at: function.span,
    }
}

fn refused(
    source: &Source,
    function: &Function,
    name: String,
    at: Option<miette::SourceSpan>,
    why: &'static str,
) -> Error {
    Error::Capability {
        function: function.name,
        name,
        why,
        src: source.named(),
        at,
    }
}
