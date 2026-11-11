//! What a workflow says about itself, read without running it.

use almanac::effect::EffectKind;
use serde::de::value::{self, StrDeserializer};
use serde::de::{Error as _, Unexpected};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Declaration {
    pub name: String,
    /// The event that starts a run.
    pub on: String,
    pub intent: String,
    /// The effects a run may produce. A run refuses any other.
    #[serde(deserialize_with = "effect_kinds")]
    pub may: Vec<EffectKind>,
    pub runs_per_hour: u32,
    /// The catalog scan that finds this workflow's events beside its webhooks, if any.
    #[serde(default)]
    pub scan: Option<String>,
    #[serde(default)]
    pub agents: BTreeMap<String, Agent>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub prompt: String,
    #[serde(default)]
    pub vars: Vec<String>,
}

/// An effect as a workflow names it, `:attest` for Almanac's `Attest`, read by `EffectKind`'s own
/// `Deserialize` so the set stays Almanac's.
pub fn effect_kind(name: &str) -> Option<EffectKind> {
    variant(name)
}

/// One of Almanac's unit variants as a workflow names it: `:failure` for `Failure`.
pub fn variant<T: for<'de> Deserialize<'de>>(name: &str) -> Option<T> {
    let mut chars = name.chars();
    let first = chars.next().filter(char::is_ascii_lowercase)?;
    let capitalized = first.to_ascii_uppercase().to_string() + chars.as_str();
    T::deserialize(StrDeserializer::<value::Error>::new(&capitalized)).ok()
}

fn effect_kinds<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<EffectKind>, D::Error> {
    let names = Vec::<String>::deserialize(deserializer)?;
    let kind = |name: &String| {
        effect_kind(name).ok_or_else(|| {
            D::Error::invalid_value(Unexpected::Str(name), &"an effect such as :attest")
        })
    };
    names.iter().map(kind).collect()
}
