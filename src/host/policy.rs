//! Running `policy`: the half of a workflow that decides.
//!
//! `policy` runs in a fresh [`Sandbox`] on the facts `gather` returned, handed a constructor for
//! each effect it destructured and nothing else: no reads, no clock, no randomness. Each
//! constructor checks its arguments and returns an opaque [`Decision`], so a policy cannot make an
//! effect it was not handed or one Almanac would refuse. `push` answers `nil` and the reason, such
//! as `:empty` or `:denied`, for a patch the patch policy refuses.
//!
//! Every Lua string is clean: text from outside reaches a script only as an opaque handle (see
//! [`super::taint`]). So a string a policy builds may become a `Title`, and an untrusted handle may
//! become an effect's body, where it stays `Untrusted`.

use super::declaration::variant;
use super::fennel::Source;
use super::gather::failed;
use super::sandbox::Sandbox;
use super::taint;
use crate::Error;
use almanac::effect::{AttestState, EffectKind};
use almanac::patch::{Checked, DEFAULT_POLICY, Patch, Rejection};
use almanac::taint::{Sha, Slug, Untrusted};
use mlua::{AnyUserData, Lua, LuaSerdeExt as _, Table, UserData, Value};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Data;
use std::collections::BTreeSet;

/// An effect a policy decided on. `context`, `until`, `message`, and `hold` are string literals
/// from the workflow's source, so the runtime can hold them for as long as it runs.
/// It serializes as Almanac stores effects, with each body still `{"$untrusted": ...}`.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Attest {
        context: String,
        state: AttestState,
        repo: Slug,
        sha: Sha,
    },
    Await {
        until: String,
    },
    File {
        title: String,
        target: Slug,
        body: Untrusted<String>,
    },
    Ask {
        question: String,
        target: Slug,
        pull: Option<u64>,
        body: Untrusted<String>,
    },
    Push {
        repo: Slug,
        pull: u64,
        branch: Slug,
        message: String,
        hold: Option<String>,
        #[serde(serialize_with = "checked")]
        patch: Checked,
    },
}

/// A checked patch as Almanac stores one: its base, and the patch still tainted.
fn checked<S: serde::Serializer>(checked: &Checked, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::Serialize as _;
    let patch = Untrusted::new(checked.patch().clone());
    serde_json::json!({"base": checked.base(), "patch": patch}).serialize(serializer)
}

impl UserData for Decision {}

/// Text for an effect's body: a policy's own string, or untrusted text carried through.
#[derive(Deserialize)]
#[serde(untagged)]
enum Body {
    Untrusted(Untrusted<String>),
    Clean(String),
}

impl Body {
    fn untrusted(self) -> Untrusted<String> {
        match self {
            Self::Untrusted(text) => text,
            Self::Clean(text) => Untrusted::new(text),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attest {
    context: String,
    state: String,
    repo: Slug,
    sha: Sha,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    title: String,
    target: Slug,
    body: Body,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ask {
    question: String,
    target: Slug,
    #[serde(default)]
    pull: Option<u64>,
    body: Body,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Push {
    repo: Slug,
    pull: u64,
    branch: Slug,
    message: String,
    #[serde(default)]
    hold: Option<String>,
    base: Sha,
    patch: Untrusted<Patch>,
}

/// Reads a constructor's argument, naming the constructor when it does not fit.
fn argument<T: DeserializeOwned>(lua: &Lua, kind: &str, value: Value) -> mlua::Result<T> {
    lua.from_value(value).map_err(|e| {
        let message = e.to_string();
        let message = message
            .strip_prefix("deserialize error: ")
            .unwrap_or(&message);
        mlua::Error::runtime(format!("({kind} ...) {message}"))
    })
}

/// `text`, if it is one of the workflow's string literals.
fn literal(
    literals: &BTreeSet<String>,
    kind: &str,
    field: &str,
    text: String,
) -> mlua::Result<String> {
    match literals.contains(&text) {
        true => Ok(text),
        false => Err(mlua::Error::runtime(format!(
            "({kind} ...) takes its {field} as a string literal, so that it is the same on every run"
        ))),
    }
}

fn construct(
    lua: &Lua,
    kind: EffectKind,
    literals: &BTreeSet<String>,
    value: Value,
) -> mlua::Result<Result<Decision, Rejection>> {
    match kind {
        EffectKind::Attest => {
            let Attest {
                context,
                state,
                repo,
                sha,
            } = argument(lua, "attest", value)?;
            let state = variant(&state).ok_or_else(|| {
                mlua::Error::runtime(
                    "(attest ...) takes a state of :pending, :success, or :failure",
                )
            })?;
            Ok(Ok(Decision::Attest {
                context: literal(literals, "attest", "context", context)?,
                state,
                repo,
                sha,
            }))
        }
        EffectKind::Await => {
            let until: String = argument(lua, "await", value)?;
            Ok(Ok(Decision::Await {
                until: literal(literals, "await", "fact", until)?,
            }))
        }
        EffectKind::File => {
            let File {
                title,
                target,
                body,
            } = argument(lua, "file", value)?;
            Ok(Ok(Decision::File {
                title,
                target,
                body: body.untrusted(),
            }))
        }
        EffectKind::Ask => {
            let Ask {
                question,
                target,
                pull,
                body,
            } = argument(lua, "ask", value)?;
            Ok(Ok(Decision::Ask {
                question,
                target,
                pull,
                body: body.untrusted(),
            }))
        }
        EffectKind::Push => {
            let Push {
                repo,
                pull,
                branch,
                message,
                hold,
                base,
                patch,
            } = argument(lua, "push", value)?;
            // A patch is checked where it is decided on, so a policy can decide what a refusal
            // means, such as asking a person.
            let patch = match DEFAULT_POLICY.check(&patch, &base) {
                Ok(patch) => patch,
                Err(rejection) => return Ok(Err(rejection)),
            };
            let hold = hold.map(|hold| literal(literals, "push", "hold", hold));
            Ok(Ok(Decision::Push {
                repo,
                pull,
                branch,
                message: literal(literals, "push", "message", message)?,
                hold: hold.transpose()?,
                patch,
            }))
        }
    }
}

pub fn run(
    sandbox: &Sandbox,
    source: &Source,
    exports: &Table,
    granted: &[EffectKind],
    literals: &BTreeSet<String>,
    facts: &Data,
) -> Result<Vec<Decision>, Error> {
    let lua = sandbox.lua();
    let constructors = lua.create_table()?;
    for &kind in granted {
        let literals = literals.clone();
        let name = format!("{kind:?}").to_lowercase();
        // A constructor answers with its effect, or with `nil` and why it refused, as `io.open`
        // does: only `push` refuses, for a patch the patch policy does not accept.
        let constructor = lua.create_function(move |lua, value: Value| {
            Ok(match construct(lua, kind, &literals, value)? {
                Ok(decision) => (Some(decision), None),
                Err(rejection) => (None, Some(format!("{rejection:?}").to_lowercase())),
            })
        })?;
        constructors.set(name, constructor)?;
    }
    let failed = |error| failed("policy", source, error);
    let thread = sandbox.thread(exports.get("policy").map_err(failed)?)?;
    let facts = taint::into_lua(lua, facts)?;
    // A policy has no capability that yields, so one resume runs it to the end.
    let decided: Value = thread.resume((facts, constructors)).map_err(failed)?;
    if decided.is_nil() {
        return Ok(vec![]);
    }
    let decided: Vec<AnyUserData> = lua.unpack(decided).map_err(|_| Error::Run {
        function: "policy",
        message: "a policy returns a sequence of effects, such as [(attest {...})]".to_string(),
        src: source.named(),
        at: None,
    })?;
    decided
        .into_iter()
        .map(|decision| decision.take::<Decision>())
        .collect::<mlua::Result<_>>()
        .map_err(|_| Error::Run {
            function: "policy",
            message: "a policy returns only effects its constructors made".to_string(),
            src: source.named(),
            at: None,
        })
}
