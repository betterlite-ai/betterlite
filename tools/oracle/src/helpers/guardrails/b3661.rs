use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_translation::{
    BedrockChecksConfigModel, BedrockGuardrailConfigModel, BedrockGuardrailStreamParams,
    JavelinGuardrailConfigModel, LakeraV2GuardrailConfigModel, NomaGuardrailConfigModel,
    PillarGuardrailConfigModel, ZscalerAIGuardConfigModel,
};
use betterlite_guardrails::guardrail_translation::{
    StreamTransformSink, StreamingScanKey, merge_user_key_metadata, transform_user_key_metadata,
};
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
            Function::BedrockChecks => config_answer(serde_json::from_value::<
                BedrockChecksConfigModel,
            >(input.body.clone())),
            Function::BedrockGuardrail => config_answer(serde_json::from_value::<
                BedrockGuardrailConfigModel,
            >(input.body.clone())),
            Function::BedrockStreamingFromExtras => config_answer(
                BedrockGuardrailStreamParams::from_extras(input.extras.as_ref()),
            ),
            Function::ProviderConfig => provider_config(input),
            Function::ScanKey => Ok(scan_key(input)),
            Function::UserMetadata => encode(transform_user_key_metadata(
                input.user_api_key_dict.as_ref(),
            )),
            Function::MergeMetadata => merge_metadata(input),
        }
    }
}

fn provider_config(input: &Input) -> HelperResult {
    match input.model {
        Model::Bedrock => config_answer(serde_json::from_value::<BedrockGuardrailConfigModel>(
            input.body.clone(),
        )),
        Model::LakeraV2 => config_answer(serde_json::from_value::<LakeraV2GuardrailConfigModel>(
            input.body.clone(),
        )),
        Model::Pillar => config_answer(serde_json::from_value::<PillarGuardrailConfigModel>(
            input.body.clone(),
        )),
        Model::Noma => config_answer(serde_json::from_value::<NomaGuardrailConfigModel>(
            input.body.clone(),
        )),
        Model::Zscaler => config_answer(serde_json::from_value::<ZscalerAIGuardConfigModel>(
            input.body.clone(),
        )),
        Model::Javelin => config_answer(serde_json::from_value::<JavelinGuardrailConfigModel>(
            input.body.clone(),
        )),
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

fn config_answer<T, E>(result: Result<T, E>) -> HelperResult
where
    T: Serialize,
{
    match result {
        Ok(value) => encode(value),
        Err(_) => Ok(json!({"accepted": false})),
    }
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
