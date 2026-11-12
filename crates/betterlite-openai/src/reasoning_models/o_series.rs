use betterlite_models::model_capability_predicates::{CapabilityRequest, NamedCapability};
use serde_json::Value;

use super::{
    DropParams, ProviderResolution, ReasoningCapabilities, ReasoningError, copy_supported,
    is_temperature_one,
};
use crate::chat::{ChatMessage, Role};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

const ALWAYS_UNSUPPORTED: [&str; 5] = [
    "logprobs",
    "top_p",
    "presence_penalty",
    "frequency_penalty",
    "top_logprobs",
];

pub struct Mapping<'request> {
    pub model: &'request str,
    pub non_default_params: &'request mut Object,
    pub optional_params: &'request mut Object,
    pub drop_params: DropParams,
    pub catalog: &'request CatalogCapabilities,
}

pub async fn supported_params<C, R>(
    capabilities: &mut C,
    resolver: &mut R,
    model: &str,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str>
where
    C: ReasoningCapabilities,
    R: ProviderResolution,
{
    let mut params = OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
    .supported_params(model, catalog);
    if !params.contains(&"reasoning_effort") {
        params.push("reasoning_effort");
    }
    let unsupported = unsupported_params(capabilities, resolver, model).await;
    params.retain(|param| !unsupported.contains(param));
    params
}

async fn unsupported_params<C, R>(
    capabilities: &mut C,
    resolver: &mut R,
    model: &str,
) -> Vec<&'static str>
where
    C: ReasoningCapabilities,
    R: ProviderResolution,
{
    let mut unsupported = ALWAYS_UNSUPPORTED.to_vec();
    let resolved_target = resolver.resolve_provider(model);
    let lookup_model = resolved_target
        .as_ref()
        .map_or(model, |target| target.model.as_str());
    let provider = resolved_target.as_ref().map_or_else(
        || {
            tracing::debug!(
                model,
                "no provider could be resolved for the o-series supported parameter check, so it falls back to openai"
            );
            String::from("openai")
        },
        |target| target.provider.clone(),
    );
    let request = CapabilityRequest {
        model: lookup_model,
        custom_llm_provider: Some(provider.as_str()),
    };
    let function_calling = capabilities
        .supports_factory(&request, NamedCapability::FunctionCalling.key())
        .await;
    if !function_calling {
        unsupported.extend(["tools", "tool_choice", "function_call", "functions"]);
    }
    let parallel_calls = capabilities
        .supports_factory(&request, NamedCapability::ParallelCalls.key())
        .await;
    if !parallel_calls {
        unsupported.push("parallel_tool_calls");
    }
    if !capabilities.supports_response_schema(&request).await {
        unsupported.push("response_format");
    }
    unsupported
}

pub async fn map_params<C, R>(
    capabilities: &mut C,
    resolver: &mut R,
    mapping: &mut Mapping<'_>,
) -> Result<(), ReasoningError>
where
    C: ReasoningCapabilities,
    R: ProviderResolution,
{
    if let Some(value) = mapping.non_default_params.remove("max_tokens") {
        mapping
            .optional_params
            .insert(String::from("max_completion_tokens"), value);
    }
    if let Some(temperature) = mapping.non_default_params.remove("temperature")
        && !temperature.is_null()
    {
        apply_temperature(mapping, temperature)?;
    }
    let supported = supported_params(capabilities, resolver, mapping.model, mapping.catalog).await;
    copy_supported(
        mapping.non_default_params,
        mapping.optional_params,
        &supported,
    );
    Ok(())
}

fn apply_temperature(mapping: &mut Mapping<'_>, temperature: Value) -> Result<(), ReasoningError> {
    if is_temperature_one(&temperature) {
        mapping
            .optional_params
            .insert(String::from("temperature"), temperature);
        return Ok(());
    }
    if mapping.drop_params.global || mapping.drop_params.request {
        return Ok(());
    }
    Err(ReasoningError::UnsupportedTemperature {
        model: mapping.model.to_owned(),
        temperature,
    })
}

pub async fn transform_messages<C>(
    capabilities: &mut C,
    model: &str,
    messages: &[ChatMessage],
) -> Vec<ChatMessage>
where
    C: ReasoningCapabilities,
{
    let request = CapabilityRequest {
        model,
        custom_llm_provider: Some("openai"),
    };
    let supports_system_messages = capabilities
        .supports_factory(&request, NamedCapability::SystemMessages.key())
        .await;
    replaced_system_messages(messages, supports_system_messages)
}

fn replaced_system_messages(
    messages: &[ChatMessage],
    supports_system_messages: bool,
) -> Vec<ChatMessage> {
    let mut transformed = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role == Role::System && !supports_system_messages {
            transformed.push(ChatMessage {
                role: Role::User,
                content: message.content.clone(),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                passthrough: indexmap::IndexMap::new(),
            });
        } else {
            transformed.push(message.clone());
        }
    }
    transformed
}
