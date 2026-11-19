use std::collections::BTreeSet;

use betterlite_anthropic::message_replay::cache_control::normalize_cache_control;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::api::anthropic_beta_filter::Headers;
use crate::api::anthropic_beta_filter::error::BetaFilterError;
use crate::api::anthropic_beta_filter::native_injection::{NativeRequest, inject_native_betas};
use crate::api::messages_native_prepare::url::MESSAGES_PATH;
use crate::api::messages_native_transform::capabilities::ThinkingCapabilities;
use crate::api::messages_native_transform::error::TransformError;
use crate::api::messages_native_transform::request::{TransformRequest, transform_native_request};

mod tests;

pub const DEFAULT_ANTHROPIC_API_VERSION: &str = "2023-06-01";
const GENERIC_PROVIDER: &str = "openai_like";

#[derive(Debug, Error)]
pub enum JsonMessagesPolicyError {
    #[error("api_base is required to forward Anthropic /v1/messages to a native endpoint")]
    BaseRequired,
    #[error(transparent)]
    Beta(#[from] BetaFilterError),
    #[error(transparent)]
    Transform(#[from] TransformError),
}

// LiteLLM's native classes resolve the key, the base and the beta headers from these call values
pub struct MessagesCredentials<'call> {
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub global_api_key: Option<&'call SecretString>,
    pub lookup: &'call dyn EnvironmentLookup,
}

// strip every trailing slash, keep a complete /v1/messages, otherwise drop one final /v1 and append the path
fn complete_messages_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with(MESSAGES_PATH) {
        return base.to_owned();
    }
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}{MESSAGES_PATH}")
}

// auth, version and content type are matched case-insensitively and injected in lowercase
fn default_anthropic_headers(mut headers: Headers, authorization: Option<SecretString>) -> Headers {
    let present: BTreeSet<String> = headers
        .keys()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    if let Some(key) = authorization
        && !present.contains("authorization")
        && !present.contains("x-api-key")
    {
        headers.insert(
            "authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    if !present.contains("anthropic-version") {
        headers.insert(
            "anthropic-version".to_owned(),
            DEFAULT_ANTHROPIC_API_VERSION.to_owned(),
        );
    }
    if !present.contains("content-type") {
        headers.insert("content-type".to_owned(), "application/json".to_owned());
    }
    headers
}

// the JSON classes lowercase every anthropic-beta spelling before B960 synthesizes the merged header
fn synthesize_betas(
    headers: Headers,
    optional_params: &Object,
    messages: &[Json],
) -> Result<Headers, JsonMessagesPolicyError> {
    let mut normalized: Headers = Headers::new();
    let mut beta: Option<String> = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("anthropic-beta") {
            beta = Some(value);
        } else {
            normalized.insert(name, value);
        }
    }
    if let Some(value) = beta {
        normalized.insert("anthropic-beta".to_owned(), value);
    }
    let merged = inject_native_betas(
        normalized,
        &NativeRequest {
            provider: "anthropic",
            optional_params,
            messages,
        },
    )?;
    Ok(merged)
}

// the generic OpenAILikeAnthropicMessagesConfig: the caller's base is required and its key is used as given
pub fn generic_complete_url(api_base: Option<&str>) -> Result<String, JsonMessagesPolicyError> {
    match api_base.filter(|value| !value.is_empty()) {
        Some(base) => Ok(complete_messages_url(base)),
        None => Err(JsonMessagesPolicyError::BaseRequired),
    }
}

pub fn generic_validate_environment(
    headers: Headers,
    api_key: Option<&SecretString>,
    optional_params: &Object,
    messages: &[Json],
) -> Result<Headers, JsonMessagesPolicyError> {
    let authorization = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned();
    synthesize_betas(
        default_anthropic_headers(headers, authorization),
        optional_params,
        messages,
    )
}

pub async fn generic_transform_request<Capabilities>(
    capabilities: &mut Capabilities,
    model: &str,
    messages: Vec<Json>,
    optional_params: Object,
) -> Result<Object, JsonMessagesPolicyError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    let body = transform_native_request(
        capabilities,
        TransformRequest {
            model,
            messages,
            optional_params,
            custom_llm_provider: GENERIC_PROVIDER,
            strips_billing_metadata: false,
        },
    )
    .await?;
    Ok(normalize_cache_control(&body))
}

// JSONProviderAnthropicMessagesConfig: the slug's env vars and the descriptor's constraints drive the request
#[derive(Clone, Debug)]
pub struct JsonMessagesPolicy {
    descriptor: ProviderDescriptor,
}

impl JsonMessagesPolicy {
    #[must_use]
    pub const fn new(descriptor: ProviderDescriptor) -> Self {
        Self { descriptor }
    }

    #[must_use]
    pub fn for_provider(registry: &JSONProviderRegistry, provider: ProviderId) -> Option<Self> {
        registry.providers.get(&provider).cloned().map(Self::new)
    }

    #[must_use]
    pub const fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn provider(&self) -> ProviderId {
        self.descriptor.provider
    }

    // a deployment's cache_control_ttl defaults to false, and the JSON subclass reads its descriptor's constraints
    #[must_use]
    pub fn cache_control_ttl(&self) -> bool {
        self.descriptor
            .config
            .constraints
            .cache_control_ttl
            .unwrap_or(false)
    }

    // explicit, then the declared api_base_env, then the descriptor's base_url
    #[must_use]
    pub fn resolve_base(&self, credentials: &MessagesCredentials<'_>) -> String {
        let config = &self.descriptor.config;
        credentials
            .api_base
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                config
                    .api_base_env
                    .as_deref()
                    .and_then(|name| credentials.lookup.lookup(name))
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| config.base_url.clone())
    }

    // explicit, then the descriptor's api_key_env, then litellm.api_key
    #[must_use]
    pub fn resolve_api_key(&self, credentials: &MessagesCredentials<'_>) -> Option<SecretString> {
        credentials
            .api_key
            .filter(|key| !key.expose_secret().is_empty())
            .cloned()
            .or_else(|| {
                credentials
                    .lookup
                    .lookup(&self.descriptor.config.api_key_env)
                    .filter(|value| !value.is_empty())
                    .map(SecretString::from)
            })
            .or_else(|| {
                credentials
                    .global_api_key
                    .filter(|key| !key.expose_secret().is_empty())
                    .cloned()
            })
    }

    pub fn complete_url(
        &self,
        credentials: &MessagesCredentials<'_>,
    ) -> Result<String, JsonMessagesPolicyError> {
        let base = self.resolve_base(credentials);
        if base.is_empty() {
            return Err(JsonMessagesPolicyError::BaseRequired);
        }
        Ok(complete_messages_url(&base))
    }

    pub fn validate_environment(
        &self,
        headers: Headers,
        credentials: &MessagesCredentials<'_>,
        optional_params: &Object,
        messages: &[Json],
    ) -> Result<Headers, JsonMessagesPolicyError> {
        let authorization = self.resolve_api_key(credentials);
        synthesize_betas(
            default_anthropic_headers(headers, authorization),
            optional_params,
            messages,
        )
    }

    // B962 does the native preparation and billing strip, then B571 reduces cache hints unless ttl is on
    pub async fn transform_request<Capabilities>(
        &self,
        capabilities: &mut Capabilities,
        model: &str,
        messages: Vec<Json>,
        optional_params: Object,
    ) -> Result<Object, JsonMessagesPolicyError>
    where
        Capabilities: ThinkingCapabilities + Send,
    {
        let provider = self.descriptor.provider.to_string();
        let body = transform_native_request(
            capabilities,
            TransformRequest {
                model,
                messages,
                optional_params,
                custom_llm_provider: &provider,
                strips_billing_metadata: true,
            },
        )
        .await?;
        if self.cache_control_ttl() {
            Ok(body)
        } else {
            Ok(normalize_cache_control(&body))
        }
    }
}
