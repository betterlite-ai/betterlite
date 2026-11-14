use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;

use crate::case::UpstreamAnswer;
use crate::helpers::HelperResult;

mod ops;
mod support;

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsFields {
    #[serde(default)]
    guardrail_name: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    fail_on_error: Option<bool>,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    buffer_until_moderated: Option<bool>,
    #[serde(default)]
    buffer_release_on_scan: Option<bool>,
    #[serde(default)]
    end_of_stream_only: Option<bool>,
    #[serde(default)]
    sampling_rate: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyInput {
    settings: SettingsFields,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    inputs: GuardrailApiInputs,
    #[serde(default)]
    request_data: BTreeMap<String, GuardrailValue>,
    #[serde(default = "default_input_type")]
    input_type: String,
    #[serde(default)]
    answer: Option<UpstreamAnswer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstructInput {
    settings: SettingsFields,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamingInput {
    settings: SettingsFields,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    params: Value,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Apply(ApplyInput),
    Construct(ConstructInput),
    StreamingParams(StreamingInput),
}

fn default_input_type() -> String {
    "request".to_owned()
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Apply(args) => ops::apply(args),
            Input::Construct(args) => ops::construct(args),
            Input::StreamingParams(args) => ops::streaming_params(args),
        }
    }
}
