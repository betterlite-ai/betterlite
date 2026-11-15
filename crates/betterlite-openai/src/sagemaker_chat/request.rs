use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object as Body};

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, ChatContentError, Mode, RefuseDownloads};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use crate::sagemaker_completion::error::SagemakerError;

pub const PROVIDER: &str = "sagemaker_chat";
// SagemakerChatConfig extends OpenAIGPTConfig, so the exact-OpenAI parameter extensions stay off
pub const FAMILY: ConfigFamily = ConfigFamily::Other;
// the runtime answers in the SageMaker event stream, so B5840 wraps it in the custom stream wrapper
pub const HAS_CUSTOM_STREAM_WRAPPER: bool = true;
// the shared chat handler leaves stream out of the body and keeps it in the URL
pub const SUPPORTS_STREAM_PARAM_IN_BODY: bool = false;

const INFERENCE_COMPONENT_HEADER: &str = "X-Amzn-SageMaker-Inference-Component";
const MODEL_ID: &str = "model_id";

pub struct TransformInput<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Body,
    pub hf_model_name: Option<&'request str>,
}

// LiteLLM's get_error_class builds the shared SagemakerError from the reply
#[must_use]
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: Option<Headers>,
) -> SagemakerError {
    SagemakerError {
        status_code,
        message: message.to_owned(),
        headers,
    }
}

#[must_use]
pub fn supported_openai_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
    let config = OpenAiConfig {
        family: FAMILY,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    };
    config.supported_params(model, catalog)
}

pub fn map_openai_params(
    model: &str,
    requested: &Object,
    existing: &mut Object,
    catalog: &CatalogCapabilities,
) {
    let config = OpenAiConfig {
        family: FAMILY,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    };
    config.map_params(model, requested, existing, catalog);
}

#[must_use]
pub fn validate_environment(headers: Headers, optional_params: &Body) -> Headers {
    // the component header is added only for a string model_id; anything else leaves the headers as given
    let Some(Json::Text(component)) = optional_params.get(MODEL_ID) else {
        return headers;
    };
    let mut merged = headers;
    merged.insert(INFERENCE_COMPONENT_HEADER.to_owned(), component.clone());
    merged
}

pub fn transform_request(input: &TransformInput<'_>) -> Result<Body, ChatContentError> {
    let mut body = build_body(input, PROVIDER)?;
    if let Some(name) = input.hf_model_name {
        // merging a dict replaces the value in place, so the model key keeps its position
        body.insert(String::from("model"), Json::Text(name.to_owned()));
    }
    Ok(body)
}

pub(super) fn build_body(
    input: &TransformInput<'_>,
    provider: &'static str,
) -> Result<Body, ChatContentError> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let content = ChatContent {
        model: input.model,
        messages: input.messages,
        optional_params: input.optional_params,
        provider: Some(provider),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    content.transform(Mode::Sync)?.to_body()
}

mod tests;
