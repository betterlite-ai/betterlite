pub mod azure;
pub mod error;
pub mod fields;
mod prepare;
pub mod provider;

mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::drop_params::normalize_drop_params;
use betterlite_models::decisions::{Extra, ir};
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, ResolvedModel, resolve};
use secrecy::{ExposeSecret as _, SecretString};

use self::azure::AzureAiConfig;
use self::error::PrepareError;
use self::fields::{RequestFields, RequestFormat};
use self::prepare::{build_ir, credentials, log_update, transform_body};
use self::provider::ProviderConfig;
use crate::credentials::LiteLlmGlobals;
use crate::decisions::providers::{DecisionsProvider, supported_decisions_providers};
use crate::decisions::transformation::wire::ProviderRequestWire;
use crate::openai_base_config::Headers;

// prepare_call's inputs: LiteLLM's arguments plus the resolution and provider seams betterlite injects.
pub struct PrepareCall<'call, 'logging> {
    pub model: &'call str,
    pub fields: RequestFields,
    pub safety_identifier: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<&'call str>,
    pub extra_headers: Option<&'call Headers>,
    pub kwargs: Extra,
    pub global_drop_params: bool,
    pub globals: LiteLlmGlobals,
    pub azure_ai: AzureAiConfig,
    pub resolution: &'call ResolutionContext<'call>,
    pub logging_obj: Option<&'logging Mutex<LoggingState>>,
}

impl fmt::Debug for PrepareCall<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("PrepareCall");
        let api_key = self.api_key.as_ref().map(|_key| "<redacted>");
        debug.field("model", &self.model);
        debug.field("custom_llm_provider", &self.custom_llm_provider);
        debug.field("safety_identifier", &self.safety_identifier);
        debug.field("api_key", &api_key);
        debug.field("api_base", &self.api_base);
        debug.field("timeout", &self.timeout);
        debug.finish_non_exhaustive()
    }
}

// The frozen call B3514 sends: LiteLLM's DecisionsCall with owned, typed values in place of its dict fields.
pub struct DecisionsCall<'logging> {
    pub model: String,
    pub requested_model: String,
    pub custom_llm_provider: String,
    pub provider_config: ProviderConfig,
    pub request: RequestFormat,
    pub ir_request: ir::DecisionRequest,
    pub body: ProviderRequestWire,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub logging_obj: Option<&'logging Mutex<LoggingState>>,
    pub headers: Headers,
    pub timeout: Option<Duration>,
    pub litellm_params: Extra,
}

impl fmt::Debug for DecisionsCall<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecisionsCall")
            .field("model", &self.model)
            .field("requested_model", &self.requested_model)
            .field("custom_llm_provider", &self.custom_llm_provider)
            .field("request", &self.request)
            .field("ir_request", &self.ir_request)
            .field("body", &self.body)
            .field("api_base", &self.api_base)
            .field("api_key", &self.api_key.as_ref().map(|_key| "<redacted>"))
            .field("headers", &self.headers)
            .field("timeout", &self.timeout)
            .field("litellm_params", &self.litellm_params)
            .finish_non_exhaustive()
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "prepare_call keeps LiteLLM's one ordered pipeline: resolve, select, validate, credentials, IR, transform, log"
)]
pub fn prepare_call<'logging>(
    call: PrepareCall<'_, 'logging>,
) -> Result<DecisionsCall<'logging>, PrepareError> {
    let resolved = resolve_model(&call)?;
    let provider = decisions_provider(&resolved)?;
    let environment = call.resolution.environment;
    let config = ProviderConfig::select(provider, call.globals, environment, call.azure_ai);
    let selection = Selection {
        model: call.model,
        environment,
        provider,
        config,
    };
    let canonical = canonical_model(&selection, &resolved.model)?;
    let dropped = call.global_drop_params
        || normalize_drop_params(call.kwargs.get("drop_params")) == Some(true);
    let request = validate_fields(&selection, call.fields, call.safety_identifier)?;
    let resolved_credentials = credentials(
        &selection,
        CredentialInput {
            api_base: call.api_base,
            api_key: call.api_key,
            dynamic_api_base: resolved.api_base.as_deref(),
            dynamic_api_key: resolved.api_key.as_ref(),
        },
    )?;
    let ir_request = build_ir(&selection, &request, call.safety_identifier, dropped)?;
    let body = transform_body(&selection, &canonical, &ir_request)?;
    if let Some(logging_obj) = call.logging_obj {
        log_update(
            &selection,
            logging_obj,
            &call.kwargs,
            &resolved_credentials.api_base,
            &canonical,
        )?;
    }
    Ok(DecisionsCall {
        model: canonical,
        requested_model: call.model.to_owned(),
        custom_llm_provider: provider.as_str().to_owned(),
        provider_config: selection.config,
        request,
        ir_request,
        body,
        api_base: resolved_credentials.api_base,
        api_key: resolved_credentials.api_key,
        logging_obj: call.logging_obj,
        headers: call.extra_headers.cloned().unwrap_or_default(),
        timeout: call.timeout,
        litellm_params: call.kwargs,
    })
}

// Requests with the other format's field go through the other endpoint: systemone rejects input, decisions takes it.
pub fn reject_other_format(
    model: &str,
    kwargs: &Extra,
    field_name: &str,
    other_function: &str,
) -> Result<(), PrepareError> {
    if !kwargs.contains_key(field_name) {
        return Ok(());
    }
    Err(PrepareError::OtherFormat {
        model: model.to_owned(),
        field: field_name.to_owned(),
        other_function: other_function.to_owned(),
    })
}

struct Selection<'call> {
    model: &'call str,
    environment: &'call BTreeMap<String, String>,
    provider: DecisionsProvider,
    config: ProviderConfig,
}

struct CredentialInput<'call> {
    api_base: Option<&'call str>,
    api_key: Option<&'call SecretString>,
    dynamic_api_base: Option<&'call str>,
    dynamic_api_key: Option<&'call SecretString>,
}

fn resolve_model(call: &PrepareCall<'_, '_>) -> Result<ResolvedModel, PrepareError> {
    resolve(
        &ResolutionRequest {
            model: call.model,
            custom_llm_provider: call.custom_llm_provider,
            api_base: call.api_base,
            api_key: call.api_key,
        },
        call.resolution,
    )
    .map_err(|source| PrepareError::Resolution {
        model: call.model.to_owned(),
        source,
    })
}

// LiteLLM raises with the model _provider_config was handed, which is the upstream model, not the requested one
fn decisions_provider(resolved: &ResolvedModel) -> Result<DecisionsProvider, PrepareError> {
    let slug = resolved.provider.slug();
    DecisionsProvider::from_provider_name(slug).ok_or_else(|| PrepareError::UnknownProvider {
        model: resolved.model.clone(),
        provider: slug.to_owned(),
        supported: supported_decisions_providers().join(", "),
    })
}

fn canonical_model(
    selection: &Selection<'_>,
    upstream_model: &str,
) -> Result<String, PrepareError> {
    if upstream_model.is_empty() {
        return Err(PrepareError::ModelRequired {
            model: selection.model.to_owned(),
            provider: selection.provider.as_str().to_owned(),
        });
    }
    Ok(selection.config.ops().canonical_model(upstream_model))
}

fn validate_fields(
    selection: &Selection<'_>,
    fields: RequestFields,
    safety_identifier: Option<&str>,
) -> Result<RequestFormat, PrepareError> {
    fields
        .validate(safety_identifier)
        .map_err(|source| PrepareError::InvalidRequest {
            model: selection.model.to_owned(),
            provider: selection.provider.as_str().to_owned(),
            source,
        })
}

fn present_secret(value: Option<&SecretString>) -> Option<&str> {
    match value {
        Some(key) if !key.expose_secret().is_empty() => Some(key.expose_secret()),
        None | Some(_) => None,
    }
}
