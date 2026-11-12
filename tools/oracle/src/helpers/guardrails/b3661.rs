use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_translation::provider_options::{
    JavelinGuardrailConfigModel, LakeraV2GuardrailConfigModel, NomaGuardrailConfigModel,
    PillarGuardrailConfigModel, ZscalerAIGuardConfigModel,
};
use betterlite_config::sections::guardrail_translation::{
    BedrockChecksConfigModel, BedrockGuardrailConfigModel, BedrockGuardrailStreamParams,
};
use betterlite_guardrails::guardrail_translation::{
    StreamTransformSink, StreamingScanKey, merge_user_key_metadata, transform_user_key_metadata,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[path = "b3661/guardrail_provider_options/tests.rs"]
mod guardrail_provider_options;

pub(crate) struct Op;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    BedrockChecks,
    BedrockGuardrail,
    BedrockStreamingFromExtras,
    ProviderConfig,
    ScanKey,
    UserMetadata,
    MergeMetadata,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Model {
    Bedrock,
    LakeraV2,
    Pillar,
    Noma,
    Zscaler,
    Javelin,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    model: Model,
    body: Value,
    extras: Option<BTreeMap<String, GuardrailValue>>,
    request_data: BTreeMap<String, GuardrailValue>,
    user_api_key_dict: Option<BTreeMap<String, GuardrailValue>>,
    key: StreamingScanKey,
    other_key: StreamingScanKey,
    sink_mutated: BTreeMap<i64, String>,
    sink_holdback: BTreeMap<i64, i64>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::BedrockChecks => deserialize_config::<BedrockChecksConfigModel>(&input.body),
            Function::BedrockGuardrail => {
                deserialize_config::<BedrockGuardrailConfigModel>(&input.body)
            }
            Function::BedrockStreamingFromExtras => {
                BedrockGuardrailStreamParams::from_extras(input.extras.as_ref())
                    .map_or_else(|_| Ok(json!({"accepted": false})), encode)
            }
            Function::ProviderConfig => match input.model {
                Model::Bedrock => deserialize_config::<BedrockGuardrailConfigModel>(&input.body),
                Model::LakeraV2 => deserialize_config::<LakeraV2GuardrailConfigModel>(&input.body),
                Model::Pillar => deserialize_config::<PillarGuardrailConfigModel>(&input.body),
                Model::Noma => deserialize_config::<NomaGuardrailConfigModel>(&input.body),
                Model::Zscaler => deserialize_config::<ZscalerAIGuardConfigModel>(&input.body),
                Model::Javelin => deserialize_config::<JavelinGuardrailConfigModel>(&input.body),
            },
            Function::ScanKey => Ok(scan_key(input)),
            Function::UserMetadata => encode(transform_user_key_metadata(
                input.user_api_key_dict.as_ref(),
            )),
            Function::MergeMetadata => merge_metadata(input),
        }
    }
}

fn scan_key(input: &Input) -> Value {
    let mut sink = StreamTransformSink::default();
    for (index, text) in &input.sink_mutated {
        sink.mutated_text_per_choice.insert(*index, text.clone());
    }
    for (index, holdback) in &input.sink_holdback {
        sink.holdback_per_choice.insert(*index, *holdback);
    }
    json!({
        "equal": input.key == input.other_key,
        "has_nothing_to_scan": input.key.has_nothing_to_scan(),
        "mutated": sink.mutated_text_per_choice,
        "holdback": sink.holdback_per_choice,
    })
}

fn merge_metadata(input: &Input) -> HelperResult {
    let mut request_data = input.request_data.clone();
    merge_user_key_metadata(&mut request_data, input.user_api_key_dict.as_ref());
    encode(request_data)
}

fn deserialize_config<T>(body: &Value) -> HelperResult
where
    T: DeserializeOwned + Serialize,
{
    serde_json::from_value::<T>(body.clone())
        .map_or_else(|_| Ok(json!({"accepted": false})), encode)
}

fn encode<T>(value: T) -> HelperResult
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}
