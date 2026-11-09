//! What a workflow says about itself, read without running it.

use almanac::effect::EffectKind;
use serde::de::value::StrDeserializer;
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

/// Effects as a workflow names them, `:attest` for Almanac's `Attest`, read by `EffectKind`'s own
/// `Deserialize` so the set stays Almanac's.
fn effect_kinds<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<EffectKind>, D::Error> {
    let names = Vec::<String>::deserialize(deserializer)?;
    names
        .iter()
        .map(|name| {
            let mut chars = name.chars();
            let first = chars
                .next()
                .filter(char::is_ascii_lowercase)
                .ok_or_else(|| {
                    D::Error::invalid_value(
                        Unexpected::Str(name),
                        &"a lowercase effect, such as :attest",
                    )
                })?;
            let capitalized = first.to_ascii_uppercase().to_string() + chars.as_str();
            EffectKind::deserialize(StrDeserializer::<D::Error>::new(&capitalized))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn may(names: serde_json::Value) -> Result<Vec<EffectKind>, String> {
        effect_kinds(names).map_err(|e| e.to_string())
    }

    #[test]
    fn each_effect_is_almanacs_own() {
        let names = json!(["attest", "await", "file", "ask", "push"]);
        let kinds = [
            EffectKind::Attest,
            EffectKind::Await,
            EffectKind::File,
            EffectKind::Ask,
            EffectKind::Push,
        ];
        assert_eq!(may(names).unwrap(), kinds);
    }

    #[test]
    fn effects_are_named_in_lowercase() {
        for name in ["Attest", "", "1attest"] {
            let err = may(json!([name])).unwrap_err();
            assert!(err.contains("a lowercase effect"), "{err}");
        }
        assert!(
            may(json!(["merge"]))
                .unwrap_err()
                .contains("unknown variant `Merge`")
        );
        assert!(may(json!("attest")).is_err());
    }
}
