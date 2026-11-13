use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object as PromptObject;
use betterlite_prompts::message_content::{Message, convert_message_content_lists};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::LocalServerError;
use super::{ProviderEnvironment, ProviderInfo, resolve_api_base, resolve_api_key};
use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, Mode, UrlDownloader};
use crate::openai_base_config::{CatalogCapabilities, Object, OpenAiConfig};

pub const DEFAULT_API_BASE: &str = "http://localhost:22088/engines/llama.cpp";
const PLACEHOLDER_KEY: &str = "dummy-key";

// the base config is OpenAIGPTConfig, so its family is Other and reasoning_effort stays unsupported
#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub base: OpenAiConfig,
}

#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment: &ProviderEnvironment,
) -> ProviderInfo {
    ProviderInfo {
        api_base: resolve_api_base(api_base, environment)
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        api_key: resolve_api_key(api_key, environment, PLACEHOLDER_KEY),
    }
}

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    format!("{}/v1/chat/completions", base.trim_end_matches('/'))
}

fn convert<Source, Target>(value: &Source) -> Result<Target, LocalServerError>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    let json =
        serde_json::to_value(value).map_err(|source| LocalServerError::MessageShape { source })?;
    serde_json::from_value(json).map_err(|source| LocalServerError::MessageShape { source })
}

// text-only content lists become plain strings first, then the OpenAI base transform runs on the result
pub fn transform_messages(
    model: &str,
    messages: &[ChatMessage],
    downloads: &dyn UrlDownloader,
) -> Result<Vec<ChatMessage>, LocalServerError> {
    let mut prompt_messages = messages
        .iter()
        .map(convert::<ChatMessage, Message>)
        .collect::<Result<Vec<_>, _>>()?;
    let converted = convert_message_content_lists(&mut prompt_messages)
        .iter()
        .map(convert::<Message, ChatMessage>)
        .collect::<Result<Vec<_>, _>>()?;
    let environment = BTreeMap::new();
    let transformed = ChatContent {
        model,
        messages: &converted,
        optional_params: &PromptObject::new(),
        provider: Some("docker_model_runner"),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads,
    }
    .transform(Mode::Sync)?;
    Ok(transformed.messages)
}

impl ChatConfig {
    pub fn map_openai_params(
        &self,
        model: &str,
        non_default_params: &IndexMap<String, Value>,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        let supported = self.base.supported_params(model, catalog);
        for (param, value) in non_default_params {
            match param.as_str() {
                "max_completion_tokens" => {
                    optional_params.insert(String::from("max_tokens"), value.clone());
                }
                name if supported.contains(&name) => {
                    optional_params.insert(param.clone(), value.clone());
                }
                _ => {}
            }
        }
    }
}
