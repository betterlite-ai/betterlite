use std::future::Future;

use betterlite_models::model_capability_predicates::{CapabilityRequest, NamedCapability};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::reasoning_models::ReasoningCapabilities;
use betterlite_openai::response_events::NativeStreamingSupport;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;

use super::error::ChatParamsError;
use crate::azure_chat_base_policy::messages::{
    sanitized_tools_update, transform_request as base_transform,
};

pub const O_SERIES_ROUTE: &str = "o_series/";
const UNSUPPORTED: [&str; 5] = [
    "logprobs",
    "top_p",
    "presence_penalty",
    "frequency_penalty",
    "top_logprobs",
];
const REASONING_EFFORT: &str = "reasoning_effort";
const NATIVE_STREAMING_MARKER: &str = "o3";

pub fn supported_params<Capabilities>(
    capabilities: &mut Capabilities,
    model: &str,
    catalog: &CatalogCapabilities,
) -> impl Future<Output = Vec<&'static str>> + Send
where
    Capabilities: ReasoningCapabilities + Send,
{
    async move {
        let mut params = OpenAiConfig {
            family: ConfigFamily::OpenAi,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        }
        .supported_params(model, catalog);
        let reasoning = if catalog.is_catalog_model(model) {
            let request = CapabilityRequest {
                model,
                custom_llm_provider: None,
            };
            capabilities
                .supports_factory(&request, NamedCapability::Reasoning.key())
                .await
        } else {
            true
        };
        if reasoning && !params.contains(&REASONING_EFFORT) {
            params.push(REASONING_EFFORT);
        }
        params.retain(|param| !UNSUPPORTED.contains(param));
        params
    }
}

pub async fn should_fake_stream<Capabilities>(
    capabilities: &mut Capabilities,
    model: Option<&str>,
    stream: Option<bool>,
    custom_llm_provider: Option<&str>,
) -> bool
where
    Capabilities: NativeStreamingSupport,
{
    if stream != Some(true) {
        return false;
    }
    if model.is_some_and(|name| !name.is_empty() && name.contains(NATIVE_STREAMING_MARKER)) {
        return false;
    }
    let Some(name) = model else {
        return true;
    };
    let request = CapabilityRequest {
        model: name,
        custom_llm_provider,
    };
    match capabilities.supports_native_streaming(&request).await {
        Ok(Json::Bool(true)) => false,
        Ok(
            Json::Null
            | Json::Bool(false)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        ) => true,
        Err(failure) => {
            tracing::debug!(
                error = %failure,
                model = name,
                "the Azure o-series model info lookup failed; faking the stream"
            );
            true
        }
    }
}

pub fn transform_request(
    model: &str,
    messages: &[Message],
    optional: &Object,
    openai_system_messages_first: bool,
) -> Result<Object, ChatParamsError> {
    let mut flattened = optional.clone();
    flattened.extend(sanitized_tools_update(optional));
    let stripped = model.replace(O_SERIES_ROUTE, "");
    let body = base_transform(
        &stripped,
        messages,
        &flattened,
        openai_system_messages_first,
    )?;
    Ok(body)
}
