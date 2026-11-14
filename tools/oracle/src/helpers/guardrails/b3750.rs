use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

use crate::helpers::HelperError;

mod fakes;
mod operations;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Construct(Box<ConstructArgs>),
    Request(Box<RequestArgs>),
    Output(Box<OutputArgs>),
    Stream(Box<StreamArgs>),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    api_key: Option<String>,
    api_base: Option<String>,
    inspect_embeddings: Option<bool>,
    guardrail_name: Option<String>,
    litellm_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConstructArgs {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HookArg {
    PreCall,
    Moderation,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RequestArgs {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    request_data: BTreeMap<String, GuardrailValue>,
    hook: HookArg,
    key_alias: Option<String>,
    answer: GuardrailValue,
    #[serde(default = "default_status")]
    answer_status: u16,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OutputArgs {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    request_data: BTreeMap<String, GuardrailValue>,
    output: String,
    key_alias: Option<String>,
    answer: GuardrailValue,
    #[serde(default = "default_status")]
    answer_status: u16,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamArgs {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    request_data: BTreeMap<String, GuardrailValue>,
    key_alias: Option<String>,
    #[serde(default)]
    chunks: Vec<GuardrailValue>,
    #[serde(default)]
    frames: Vec<GuardrailValue>,
}

fn default_status() -> u16 {
    200
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
