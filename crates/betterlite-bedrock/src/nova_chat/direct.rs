use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::base_helpers::provider_config;
use betterlite_openai::compatible_wire::{self, CompatibleReply, LogInput, ReplyOptions};
use betterlite_openai::openai_base_config::Object;
use betterlite_prompts::arguments::json::Json;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::error::NovaChatError;
use crate::image_call::plan::LogContext;

pub const CUSTOM_LLM_PROVIDER: ProviderId = ProviderId::AmazonNova;
pub const DEFAULT_API_BASE: &str = "https://api.nova.amazon.com/v1";
pub const RESPONSE_MODEL_PREFIX: &str = "amazon-nova/";
const API_BASE_VARIABLE: &str = "AMAZON_NOVA_API_BASE";
const API_KEY_VARIABLE: &str = "AMAZON_NOVA_API_KEY";

// AmazonNovaChatConfig.get_supported_openai_params returns this list whatever the model is.
pub const SUPPORTED_OPENAI_PARAMS: [&str; 11] = [
    "top_p",
    "temperature",
    "max_tokens",
    "max_completion_tokens",
    "metadata",
    "stop",
    "stream",
    "stream_options",
    "tools",
    "tool_choice",
    "reasoning_effort",
];

// The six AmazonNovaChatConfig constructor arguments, each absent until the operator sets it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct NovaChatArgs {
    pub max_completion_tokens: Option<i64>,
    pub max_tokens: Option<i64>,
    pub temperature: Option<i64>,
    pub top_p: Option<i64>,
    pub tools: Option<Vec<Json>>,
    pub reasoning_effort: Option<Vec<Json>>,
}

// `metadata` is a class field the constructor cannot set, so it sits outside the arguments record.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct NovaChatConfig {
    #[serde(flatten)]
    pub args: NovaChatArgs,
    pub metadata: Option<i64>,
}

impl NovaChatConfig {
    // get_config reads the class dict: the fields the constructor set, and nothing else.
    pub fn config(&self) -> Result<Object, NovaChatError> {
        match provider_config(self) {
            Ok(config) => Ok(config),
            Err(source) => Err(NovaChatError::Config(source)),
        }
    }
}

// `api_base or get_secret_str(...) or default` and `api_key or litellm.amazon_nova_api_key or get_secret_str(...) or litellm.api_key`: every empty value is falsy and falls through.
pub struct NovaEndpointRequest<'call> {
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub global_api_key: Option<&'call SecretString>,
    pub litellm_api_key: Option<&'call SecretString>,
}

#[derive(Clone, Debug)]
pub struct NovaEndpoint {
    pub api_base: String,
    pub api_key: Option<SecretString>,
}

pub fn resolve_endpoint(
    request: &NovaEndpointRequest<'_>,
    environment: &BTreeMap<String, String>,
    secrets: &SecretLookup,
) -> Result<NovaEndpoint, NovaChatError> {
    let api_base = match non_empty_text(request.api_base) {
        Some(base) => base,
        None => environment_text(secrets, environment, API_BASE_VARIABLE)?
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    };
    // The `or` chain reads the environment only when an explicit and a global key are both absent.
    let mut api_key =
        non_empty_secret(request.api_key).or_else(|| non_empty_secret(request.global_api_key));
    if api_key.is_none() {
        api_key = environment_text(secrets, environment, API_KEY_VARIABLE)?
            .map(SecretString::from)
            .or_else(|| non_empty_secret(request.litellm_api_key));
    }
    Ok(NovaEndpoint { api_base, api_key })
}

// An empty environment value is falsy in Python, so it falls through like an unset one.
fn environment_text(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
    name: &str,
) -> Result<Option<String>, NovaChatError> {
    match secrets.get_secret_str(environment, name, None)? {
        Some(secret) if !secret.expose_secret().is_empty() => {
            Ok(Some(secret.expose_secret().to_owned()))
        }
        Some(_) | None => Ok(None),
    }
}

fn non_empty_text(value: Option<&str>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text.to_owned()),
        Some(_) | None => None,
    }
}

fn non_empty_secret(value: Option<&SecretString>) -> Option<SecretString> {
    match value {
        Some(secret) if !secret.expose_secret().is_empty() => Some(secret.clone()),
        Some(_) | None => None,
    }
}

// OpenAILikeChatConfig.transform_response, then the ModelResponse names amazon-nova/<model> for cost lookup.
pub fn transform_response(
    model: &str,
    bytes: &[u8],
    options: &ReplyOptions<'_>,
    log: &mut LogContext<'_, '_>,
    input: LogInput,
) -> Result<CompatibleReply, NovaChatError> {
    let mut reply = compatible_wire::transform_response(bytes, options, log.state, log.env, input)?;
    reply.response.model = format!("{RESPONSE_MODEL_PREFIX}{model}");
    Ok(reply)
}
