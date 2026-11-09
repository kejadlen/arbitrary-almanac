//! What a workflow says about itself, read without running it.

use almanac::effect::EffectKind;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Declaration {
    pub name: String,
    /// The event that starts a run.
    pub on: String,
    pub intent: String,
    /// The effects a run may produce. A run refuses any other.
    pub may: Vec<May>,
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

/// An effect as a workflow names it: `:attest`, not `Attest`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum May {
    Attest,
    Await,
    File,
    Ask,
    Push,
}

impl From<May> for EffectKind {
    fn from(may: May) -> Self {
        match may {
            May::Attest => EffectKind::Attest,
            May::Await => EffectKind::Await,
            May::File => EffectKind::File,
            May::Ask => EffectKind::Ask,
            May::Push => EffectKind::Push,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_effect_maps_to_almanacs_own() {
        let pairs = [
            (May::Attest, EffectKind::Attest),
            (May::Await, EffectKind::Await),
            (May::File, EffectKind::File),
            (May::Ask, EffectKind::Ask),
            (May::Push, EffectKind::Push),
        ];
        for (may, kind) in pairs {
            assert_eq!(EffectKind::from(may), kind);
        }
    }
}
