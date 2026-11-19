use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// the per-minute tpm and rpm LiteLLM keeps under the "%Y-%m-%d-%H-%M" key of a deployment entry
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinuteUsage {
    #[serde(default)]
    pub tpm: i64,
    #[serde(default)]
    pub rpm: i64,
}

// one deployment's bounded latency and first-token windows and its per-minute usage
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatencySamples {
    #[serde(default)]
    pub latency: Vec<f64>,
    #[serde(default)]
    pub time_to_first_token_seconds: Vec<f64>,
    #[serde(default)]
    pub minutes: BTreeMap<String, MinuteUsage>,
}

// the "{model_group}_map" cache value: every deployment the model group has recorded a callback for
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelGroupSamples {
    #[serde(default)]
    pub deployments: BTreeMap<String, LatencySamples>,
}
