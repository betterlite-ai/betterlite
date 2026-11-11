#![expect(
    clippy::disallowed_types,
    reason = "The parameter mapping carries caller-supplied passthrough values, which no typed model covers."
)]

pub mod capabilities;
pub mod classification;
mod mapping;
mod tests;

use std::collections::BTreeMap;

use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;
use thiserror::Error;

use crate::params::capabilities::{CapabilityQuery, ParameterCapabilities};
use crate::params::classification::{ParameterClass, classify_parameter};

// The names the chat request models as typed fields; every one is a standard name LiteLLM validates.
pub const TYPED_PARAMETER_NAMES: &[&str] = &[
    "temperature",
    "top_p",
    "presence_penalty",
    "frequency_penalty",
    "n",
    "max_tokens",
    "max_completion_tokens",
    "seed",
    "stop",
    "logprobs",
    "top_logprobs",
    "user",
    "service_tier",
    "stream",
    "thinking",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    #[default]
    ChatCompletion,
    Embeddings,
    Transcription,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ChatParameters {
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize"
    )]
    pub temperature: Option<Decimal>,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize"
    )]
    pub top_p: Option<Decimal>,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize"
    )]
    pub presence_penalty: Option<Decimal>,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize"
    )]
    pub frequency_penalty: Option<Decimal>,
    #[serde(rename = "n")]
    pub choice_count: Option<u64>,
    pub max_tokens: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub seed: Option<i64>,
    pub stop: Option<Stop>,
    pub logprobs: Option<bool>,
    pub top_logprobs: Option<u64>,
    pub user: Option<String>,
    pub service_tier: Option<String>,
    pub stream: Option<bool>,
    pub thinking: Option<Thinking>,
    // every other request name travels here and classification.rs decides what it is
    #[serde(flatten, skip_serializing)]
    pub passthrough: IndexMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Stop {
    One(String),
    Many(Vec<String>),
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Thinking {
    #[serde(rename = "type")]
    pub kind: Option<ThinkingType>,
    pub budget_tokens: Option<i64>,
    pub display: Option<ThinkingDisplay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingType {
    Enabled,
    Adaptive,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingDisplay {
    Summarized,
    Omitted,
    Updates,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ParameterPolicy {
    pub drop_params: bool,
    pub additional_drop_params: Vec<String>,
    pub allowed_openai_params: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ParameterMapping {
    pub standard: BTreeMap<String, Value>,
    pub provider_fields: BTreeMap<String, Value>,
    pub dropped_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ParameterRequest<'request> {
    pub capabilities: CapabilityQuery<'request>,
    pub parameters: &'request ChatParameters,
    pub policy: &'request ParameterPolicy,
}

#[derive(Debug, Error)]
pub enum ParameterError {
    #[error("{model} does not support the parameters {names:?}")]
    Unsupported { model: String, names: Vec<String> },
    #[error("{name} is a control or content field and cannot pass through to a provider")]
    ReservedParameter { name: String },
    #[error("the supported parameters of {model} are unknown")]
    CapabilitiesAbsent { model: String },
    #[error("the request parameters could not be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("{name} must be an object")]
    NotAnObject { name: String },
    #[error("the nested drop path {path} is not supported yet")]
    NestedDropPath { path: String },
}

pub fn map_parameters(request: &ParameterRequest<'_>) -> Result<ParameterMapping, ParameterError> {
    let model = request.capabilities.model.model;
    let ParameterCapabilities::Listed(supported) =
        capabilities::supported_parameters(&request.capabilities)
    else {
        return Err(ParameterError::CapabilitiesAbsent {
            model: model.to_owned(),
        });
    };
    mapping::reject_nested_drop_paths(&request.policy.additional_drop_params)?;
    let mut state = mapping::MappingState::new(&supported, request.policy);
    for (name, value) in mapping::typed_parameters(request.parameters)? {
        state.place(name, value, false);
    }
    for (name, value) in &request.parameters.passthrough {
        if name == "extra_body" {
            state.extra_body(value, request.capabilities.call_type)?;
            continue;
        }
        match classify_parameter(name, request.capabilities.call_type) {
            ParameterClass::Reserved => {
                return Err(ParameterError::ReservedParameter { name: name.clone() });
            }
            ParameterClass::Standard => state.place(name.clone(), value.clone(), false),
            ParameterClass::ProviderPassthrough => state.place(name.clone(), value.clone(), true),
        }
    }
    state.finish(model)
}
