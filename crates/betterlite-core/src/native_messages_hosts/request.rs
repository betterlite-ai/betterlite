use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::anthropic_beta_filter::Headers;
use crate::api::anthropic_beta_filter::native_injection::NativeRequest;
use crate::api::messages_native_prepare::environment::validate_environment;

use super::error::HostError;
use super::keys::{KeySources, resolve_key};

// The parent beta updater runs under LiteLLM's default provider tag, so Tencent and MiniMax inherit it
const INHERITED_BETA_PROVIDER: &str = "anthropic";

pub struct EnvironmentRequest<'request> {
    pub headers: &'request mut Headers,
    pub explicit_key: Option<&'request SecretString>,
    pub environment_key: Option<&'request SecretString>,
    pub global_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub optional_params: &'request IndexMap<String, Json>,
    pub messages: &'request [Json],
}

pub struct HostRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<Json>,
    pub optional_params: Object,
}

// Tencent and MiniMax keep the parent's case-insensitive credential check, OAuth handling and error
pub fn validate_inherited_environment(
    request: EnvironmentRequest<'_>,
    settings: &AnthropicSettings,
) -> Result<Option<String>, HostError> {
    let headers = request.headers;
    let key = resolve_key(KeySources {
        explicit: request.explicit_key,
        environment: request.environment_key,
        global: request.global_key,
    });
    let native = NativeRequest {
        provider: INHERITED_BETA_PROVIDER,
        optional_params: request.optional_params,
        messages: request.messages,
    };
    let api_base = validate_environment(headers, key, request.api_base, &native, settings)?;
    Ok(api_base)
}
