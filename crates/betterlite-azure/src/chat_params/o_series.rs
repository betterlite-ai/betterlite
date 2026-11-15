use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::model_capability_predicates::{CapabilityRequest, NamedCapability};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{ChatContent, ChatContentError, Mode, RefuseDownloads};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::reasoning_models::ReasoningCapabilities;
use betterlite_openai::reasoning_models::o_series::transform_messages;
use betterlite_openai::response_events::NativeStreamingSupport;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_prompts::message_fields::system_runs::system_messages_first;

use super::error::ChatParamsError;
use crate::azure_chat_base_policy::messages::sanitized_tools_update;

pub const O_SERIES_ROUTE: &str = "o_series/";
const AZURE_PROVIDER: &str = "azure";
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

pub async fn transform_request<Capabilities>(
    capabilities: &mut Capabilities,
    model: &str,
    messages: &[Message],
    optional: &Object,
    openai_system_messages_first: bool,
) -> Result<Object, ChatParamsError>
where
    Capabilities: ReasoningCapabilities,
{
    let mut flattened = optional.clone();
    flattened.extend(sanitized_tools_update(optional));
    let stripped = model.replace(O_SERIES_ROUTE, "");
    // LiteLLM orders the instructions before the o-series position turns a system message into a user one
    let ordered = if openai_system_messages_first {
        system_messages_first(messages)
    } else {
        messages.to_vec()
    };
    let converted = ordered
        .iter()
        .map(chat_message)
        .collect::<Result<Vec<_>, _>>()?;
    let transformed = transform_messages(capabilities, &stripped, &converted).await;
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let content = ChatContent {
        model: &stripped,
        messages: &transformed,
        optional_params: &flattened,
        provider: Some(AZURE_PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    Ok(content.transform(Mode::Sync)?.to_body()?)
}

fn chat_message(message: &Message) -> Result<ChatMessage, ChatContentError> {
    let value = serde_json::to_value(message)
        .map_err(|source| ChatContentError::MessageShape { source })?;
    serde_json::from_value(value).map_err(|source| ChatContentError::MessageShape { source })
}
