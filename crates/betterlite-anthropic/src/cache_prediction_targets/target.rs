use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use url::Url;

use crate::cache_prediction_targets::messages_url::MessagesUrl;
use crate::credentials::{AnthropicSettings, is_anthropic_oauth_key};

const ANTHROPIC_PROVIDER: &str = "anthropic";
const API_KEY_HEADER: &str = "x-api-key";
const PUBLIC_ENDPOINTS: [&str; 2] = [
    "https://api.anthropic.com",
    "https://api.anthropic.com/v1/messages",
];
const DEPLOYMENT_OPTIONS: [&str; 16] = [
    "model",
    "api_key",
    "api_base",
    "custom_llm_provider",
    "rpm",
    "tpm",
    "timeout",
    "stream_timeout",
    "max_retries",
    "num_retries",
    "max_parallel_requests",
    "input_cost_per_token",
    "output_cost_per_token",
    "cache_read_input_token_cost",
    "cache_creation_input_token_cost",
    "cache_creation_input_token_cost_above_1hr",
];
// the only LiteLLM_Params fields whose declared default is not None, so exclude_defaults drops an explicit false
const FALSE_DEFAULT_OPTIONS: [&str; 5] = [
    "allow_client_keepalive_override",
    "merge_reasoning_content_in_choices",
    "use_in_pass_through",
    "use_litellm_proxy",
    "use_xai_oauth",
];

#[derive(Clone, Debug)]
pub struct NativePredictionTarget {
    pub model: String,
    pub api_key: SecretString,
    pub api_base: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum UnsupportedPredictionTarget {
    #[error("the deployment configures options native cache prediction cannot observe")]
    DeploymentConfiguration,
    #[error("the deployment's endpoint is not Anthropic's public messages endpoint")]
    ProviderEndpoint,
    #[error("the deployment does not resolve to the Anthropic provider")]
    Provider,
    #[error("the deployment has no usable Anthropic API key")]
    ProviderCredentials,
}

impl UnsupportedPredictionTarget {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::DeploymentConfiguration => "unsupported_deployment_configuration",
            Self::ProviderEndpoint => "unsupported_provider_endpoint",
            Self::Provider => "unsupported_provider",
            Self::ProviderCredentials => "unsupported_provider_credentials",
        }
    }
}

pub struct WireRequest<'wire> {
    pub headers: &'wire HeaderMap,
    pub url: &'wire Url,
}

// LiteLLM resolves env, provider identity and credentials inside the resolver; betterlite takes each resolved
pub fn resolve_prediction_target(
    params: &JsonObject,
    settings: &AnthropicSettings,
    resolution: &ResolutionContext<'_>,
) -> Result<NativePredictionTarget, UnsupportedPredictionTarget> {
    ensure_supported_options(params)?;
    let api_base = configured_api_base(params, settings)?;
    if !PUBLIC_ENDPOINTS.contains(&api_base.as_str()) {
        return Err(UnsupportedPredictionTarget::ProviderEndpoint);
    }
    resolve_anthropic_target(params, settings, resolution, api_base)
}

pub fn resolve_baseline_target(
    params: &JsonObject,
    settings: &AnthropicSettings,
    resolution: &ResolutionContext<'_>,
) -> Result<NativePredictionTarget, UnsupportedPredictionTarget> {
    ensure_supported_options(params)?;
    resolve_anthropic_target(
        params,
        settings,
        resolution,
        configured_api_base(params, settings)?,
    )
}

#[must_use]
pub fn supported_provider_key(api_key: &str) -> bool {
    if api_key.is_empty() {
        return false;
    }
    !is_anthropic_oauth_key(Some(api_key))
}

#[must_use]
pub fn supported_baseline_recipient<Builder>(
    target: &NativePredictionTarget,
    wire: &WireRequest<'_>,
    messages_url: &Builder,
) -> bool
where
    Builder: MessagesUrl,
{
    let Some(presented) = wire.headers.get(API_KEY_HEADER) else {
        return false;
    };
    if !bool::from(
        presented
            .as_bytes()
            .ct_eq(target.api_key.expose_secret().as_bytes()),
    ) {
        return false;
    }
    let Ok(expected) = Url::parse(&messages_url.messages_url(Some(&target.api_base))) else {
        return false;
    };
    wire.url == &expected
}

fn ensure_supported_options(params: &JsonObject) -> Result<(), UnsupportedPredictionTarget> {
    if params.iter().any(|(key, value)| {
        configured_option(key, value) && !DEPLOYMENT_OPTIONS.contains(&key.as_str())
    }) {
        return Err(UnsupportedPredictionTarget::DeploymentConfiguration);
    }
    Ok(())
}

// ports model_dump(exclude_defaults=True, exclude_none=True) over the deployment's litellm_params
fn configured_option(key: &str, value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(false) => !FALSE_DEFAULT_OPTIONS.contains(&key),
        Json::Bool(true) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            true
        }
    }
}

fn configured_api_base(
    params: &JsonObject,
    settings: &AnthropicSettings,
) -> Result<String, UnsupportedPredictionTarget> {
    match params.get("api_base") {
        Some(Json::Text(base)) if !base.is_empty() => Ok(base.to_owned()),
        None | Some(Json::Null | Json::Text(_)) => Ok(settings.api_base.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(UnsupportedPredictionTarget::ProviderEndpoint)
        }
    }
}

fn resolve_anthropic_target(
    params: &JsonObject,
    settings: &AnthropicSettings,
    resolution: &ResolutionContext<'_>,
    api_base: String,
) -> Result<NativePredictionTarget, UnsupportedPredictionTarget> {
    let model = resolved_model(params, resolution)?;
    let api_key = resolved_api_key(params, settings)?;
    Ok(NativePredictionTarget {
        model,
        api_key,
        api_base,
    })
}

fn resolved_model(
    params: &JsonObject,
    resolution: &ResolutionContext<'_>,
) -> Result<String, UnsupportedPredictionTarget> {
    let Some(Json::Text(model)) = params.get("model") else {
        return Err(UnsupportedPredictionTarget::Provider);
    };
    let custom_llm_provider = match params.get("custom_llm_provider") {
        None | Some(Json::Null) => None,
        Some(Json::Text(provider)) => Some(provider.as_str()),
        Some(_) => return Err(UnsupportedPredictionTarget::Provider),
    };
    let request = ResolutionRequest {
        model: model.as_str(),
        custom_llm_provider,
        api_base: None,
        api_key: None,
    };
    let Ok(resolved) = resolve(&request, resolution) else {
        return Err(UnsupportedPredictionTarget::Provider);
    };
    if resolved.provider.slug() != ANTHROPIC_PROVIDER {
        return Err(UnsupportedPredictionTarget::Provider);
    }
    Ok(resolved.model)
}

fn resolved_api_key(
    params: &JsonObject,
    settings: &AnthropicSettings,
) -> Result<SecretString, UnsupportedPredictionTarget> {
    let explicit = match params.get("api_key") {
        None | Some(Json::Null) => None,
        Some(Json::Text(key)) => Some(key.as_str()),
        Some(_) => return Err(UnsupportedPredictionTarget::ProviderCredentials),
    };
    let candidate = explicit
        .filter(|key| !key.is_empty())
        .map(SecretString::from)
        .or_else(|| settings.api_key.clone());
    let Some(api_key) = candidate else {
        return Err(UnsupportedPredictionTarget::ProviderCredentials);
    };
    if !supported_provider_key(api_key.expose_secret()) {
        return Err(UnsupportedPredictionTarget::ProviderCredentials);
    }
    Ok(api_key)
}
