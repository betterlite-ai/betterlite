pub(crate) mod scenario;

use std::collections::BTreeMap;

use betterlite_router::latency_observations::error::LatencyError;
use betterlite_router::latency_observations::samples::MinuteUsage;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// a wall-clock second as the case spells it, compared bit for bit so both sides answer the same JSON number
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Seconds(pub f64);

// the reply is an internally tagged enum, so serde buffers it and arbitrary_precision hands a float back as its private number map; Number reads either shape
impl<'de> Deserialize<'de> for Seconds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let number = serde_json::Number::deserialize(deserializer)?;
        number
            .as_f64()
            .map(Self)
            .ok_or_else(|| D::Error::custom("a second value must be a finite JSON number"))
    }
}

impl PartialEq for Seconds {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Seconds {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub settings: SettingsInput,
    pub steps: Vec<Step>,
}

// RoutingArgs of the lowest-latency handler the Python probe builds
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub ttl: Seconds,
    pub lowest_latency_buffer: Seconds,
    pub max_latency_list_size: i64,
    pub ttft_percentile: Option<Seconds>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Success,
    Failure,
    View,
    Score,
}

// which of litellm_params' metadata fields carries the model group, and whether the request carries litellm_metadata
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channels {
    #[default]
    Metadata,
    LitellmMetadata,
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    #[default]
    Timeout,
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleKind {
    #[default]
    Latency,
    Ttft,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Statistic {
    #[default]
    Mean,
    Percentile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageInput {
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    pub now_millis: u64,
    #[serde(default)]
    pub channels: Channels,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub metadata_group: Option<String>,
    #[serde(default)]
    pub deployment_id: Option<String>,
    #[serde(default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub stream: bool,
    // which LiteLLM callback the Python probe calls; betterlite has one production path for both
    #[serde(default)]
    pub sync: bool,
    #[serde(default)]
    pub start_millis: u64,
    #[serde(default)]
    pub elapsed_millis: u64,
    #[serde(default)]
    pub first_token_millis: Option<u64>,
    #[serde(default)]
    pub usage: Option<UsageInput>,
    #[serde(default)]
    pub failure: FailureKind,
    #[serde(default)]
    pub sample: SampleKind,
    #[serde(default)]
    pub statistic: Statistic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOutput {
    pub kind: StepKind,
    pub deployments: Option<BTreeMap<String, DeploymentView>>,
    pub score: Option<Seconds>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentView {
    pub latency: Vec<Seconds>,
    pub time_to_first_token_seconds: Vec<Seconds>,
    pub minutes: BTreeMap<String, MinuteUsage>,
}

#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error(transparent)]
    Latency(#[from] LatencyError),
    #[error("the step needs {what}")]
    Missing { what: &'static str },
    #[error("{what}")]
    Unsupported { what: &'static str },
    #[error("the time {millis} milliseconds cannot be represented as seconds")]
    TimeOverflow { millis: u64 },
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    match scenario::run(input) {
        Ok(output) => Ok(ProbeOutput::LatencyObservations(output)),
        Err(error) => Err(OperationError::InvalidRequest {
            message: error.to_string(),
        }),
    }
}
