use std::fmt;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::credentials::ANTHROPIC_VERSION;
use betterlite_anthropic::headers::request::{
    ANTHROPIC_VERSION_HEADER, CONTENT_TYPE_HEADER, JSON_CONTENT_TYPE,
};
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_core::api::anthropic_beta_filter::native_injection::{
    NativeRequest as BetaRequest, inject_native_betas,
};
use betterlite_core::api::messages_native_transform::capabilities::AnthropicCapabilities;
use betterlite_core::api::messages_native_transform::request::{
    TransformRequest, transform_native_request,
};
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::claude_platform_inputs::CLAUDE_PLATFORM_BEDROCK_ROUTE;
use crate::claude_platform_inputs::body::PlatformWarning;
use crate::model_routes::ParamMap;

use super::error::{NativeHeaderError, NativeTransformError};
use super::{
    MESSAGES_PROVIDER, WORKSPACE_HEADER, WorkspaceSources, convert, require_workspace,
    resolve_api_key,
};

pub const API_KEY_HEADER: &str = "x-api-key";

pub struct NativeHeadersRequest<'request> {
    pub model: &'request str,
    pub caller_headers: &'request IndexMap<String, String>,
    pub messages: &'request [PromptJson],
    pub optional_params: &'request IndexMap<String, PromptJson>,
    pub litellm_params: &'request IndexMap<String, Json>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub workspace: WorkspaceSources<'request>,
}

// LiteLLM returns the api_base it was handed next to the headers.
pub struct NativeHeaders {
    pub headers: IndexMap<String, String>,
    pub api_base: Option<String>,
}

// The header names stay readable; values do not, because x-api-key carries the secret.
impl fmt::Debug for NativeHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.keys().map(String::as_str).collect();
        f.debug_tuple("NativeHeaders")
            .field(&names)
            .field(&self.api_base)
            .finish()
    }
}

// validate_anthropic_messages_environment: defaults, the workspace, then the beta updater.
pub fn validate_environment(
    request: &NativeHeadersRequest<'_>,
) -> Result<NativeHeaders, NativeHeaderError> {
    let optional_params: IndexMap<String, Json> = convert::mapped(request.optional_params)?;
    let contract: ParamMap = convert::mapped(&optional_params)?;
    let workspace = require_workspace(request.model, &contract, request.workspace)?;
    let (filtered, _warnings) = convert::filtered_params(
        &optional_params,
        request.litellm_params,
        convert::FilterWarnings::Suppress,
    )?;
    let filtered_params: IndexMap<String, PromptJson> = convert::mapped(&filtered)?;
    let mut headers = request.caller_headers.clone();
    if !headers.contains_key(ANTHROPIC_VERSION_HEADER) {
        headers.insert(
            ANTHROPIC_VERSION_HEADER.to_owned(),
            ANTHROPIC_VERSION.to_owned(),
        );
    }
    if !headers.contains_key(CONTENT_TYPE_HEADER) {
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
    }
    headers.insert(WORKSPACE_HEADER.to_owned(), workspace);
    let api_key = resolve_api_key(request.api_key, request.workspace.environment);
    if let Some(key) = api_key
        && !headers.contains_key(API_KEY_HEADER)
    {
        headers.insert(API_KEY_HEADER.to_owned(), key.expose_secret().to_owned());
    }
    let headers = inject_native_betas(
        headers,
        &BetaRequest {
            provider: MESSAGES_PROVIDER,
            optional_params: &filtered_params,
            messages: request.messages,
        },
    )?;
    Ok(NativeHeaders {
        headers,
        api_base: request.api_base.map(str::to_owned),
    })
}

pub struct NativeRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<PromptJson>,
    pub optional_params: IndexMap<String, PromptJson>,
    pub litellm_params: IndexMap<String, Json>,
}

pub struct NativeSources<'context, 'environment, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    pub environment: &'context mut CapabilityEnvironment<'environment, Source, Transport>,
    pub catalogs: &'context CatalogSources<'environment>,
    pub budgets: &'context Settings,
}

pub struct NativePrepared {
    pub body: Object,
    pub warnings: Vec<PlatformWarning>,
}

// transform_anthropic_messages_request: the route prefix comes off, B2211 filters, B962 maps the body.
pub async fn transform_request<Source, Transport>(
    request: &NativeRequest<'_>,
    sources: &mut NativeSources<'_, '_, Source, Transport>,
) -> Result<NativePrepared, NativeTransformError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let optional_params: IndexMap<String, Json> = convert::mapped(&request.optional_params)?;
    let (filtered, warnings) = convert::filtered_params(
        &optional_params,
        &request.litellm_params,
        convert::FilterWarnings::Report,
    )?;
    let filtered_params: IndexMap<String, PromptJson> = convert::mapped(&filtered)?;
    let model = request
        .model
        .strip_prefix(CLAUDE_PLATFORM_BEDROCK_ROUTE)
        .unwrap_or(request.model);
    let mut capabilities = AnthropicCapabilities {
        environment: &mut *sources.environment,
        catalogs: sources.catalogs,
        budgets: sources.budgets,
        llm_provider: MESSAGES_PROVIDER,
    };
    let body = transform_native_request(
        &mut capabilities,
        TransformRequest {
            model,
            messages: request.messages.clone(),
            optional_params: filtered_params,
            custom_llm_provider: MESSAGES_PROVIDER,
            strips_billing_metadata: false,
        },
    )
    .await?;
    Ok(NativePrepared { body, warnings })
}
