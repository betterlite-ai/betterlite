use std::collections::BTreeSet;

use betterlite_models::projections::reasoning_effort::ThinkingBudgetThresholds;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::hosted_vllm::chat::messages::transform_messages;
use betterlite_openai::hosted_vllm::chat::{self, ChatConfig};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use serde::Deserialize;
use serde_json::Value;

use super::invalid;
use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportedCall {
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCall {
    model: String,
    params: Object,
    preset: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesCall {
    model: String,
    messages: Vec<ChatMessage>,
}

const fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn config() -> ChatConfig {
    ChatConfig {
        base: OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        },
        thresholds: ThinkingBudgetThresholds::default(),
    }
}

pub(super) fn supported_value(call: &SupportedCall) -> Value {
    let params = chat::supported_params(&call.model, &catalog());
    Value::Array(
        params
            .into_iter()
            .map(|name| Value::String(String::from(name)))
            .collect(),
    )
}

pub(super) fn map_value(call: &MapCall) -> Result<Value, Box<HelperError>> {
    let mut optional = call.preset.clone();
    config()
        .map_openai_params(&call.model, &call.params, &mut optional, &catalog())
        .map_err(|error| invalid(error.to_string()))?;
    Ok(Value::Object(optional))
}

pub(super) fn messages_value(call: &MessagesCall) -> Result<Value, Box<HelperError>> {
    let messages = transform_messages(&call.model, &call.messages, &RefuseDownloads)
        .map_err(|error| invalid(error.to_string()))?;
    serde_json::to_value(messages).map_err(|error| invalid(error.to_string()))
}
