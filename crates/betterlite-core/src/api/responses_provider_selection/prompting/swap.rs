use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest};
use thiserror::Error;

use super::PromptError;
use super::values::present;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::error::CredentialBoundaryError;
use crate::api::responses_provider_selection::resolution::ModelResolver;

#[derive(Debug, Error)]
pub enum PromptSwapError {
    #[error(transparent)]
    CredentialBoundary(#[from] CredentialBoundaryError),
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
}

pub struct SwapRequest<'request> {
    pub original_model: &'request str,
    pub swapped_model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub prompt_id: Option<&'request str>,
}

// LiteLLM's _resolve_prompt_swapped_provider: resolved credentials must never cross to another provider
pub fn resolve_prompt_swapped_provider(
    request: &SwapRequest<'_>,
    kwargs: &ParameterMap,
    resolver: &dyn ModelResolver,
) -> Result<String, PromptError> {
    let swapped_provider = match provider_of(resolver, request.swapped_model) {
        Ok(provider) => provider,
        Err(error) => return Err(PromptError::Swap(PromptSwapError::Resolution(error))),
    };
    if present(kwargs, "api_key").is_none() && present(kwargs, "api_base").is_none() {
        return Ok(swapped_provider);
    }
    let original_provider = match request.custom_llm_provider {
        Some(provider) => provider.to_owned(),
        None => match provider_of(resolver, request.original_model) {
            Ok(provider) => provider,
            // LiteLLM catches the BadRequestError that an unresolvable original provider raises
            Err(_unresolved) => return Ok(swapped_provider),
        },
    };
    if swapped_provider == original_provider {
        return Ok(swapped_provider);
    }
    Err(PromptError::Swap(PromptSwapError::CredentialBoundary(
        CredentialBoundaryError {
            message: credential_boundary_message(request, &original_provider, &swapped_provider),
            model: request.swapped_model.to_owned(),
            provider: swapped_provider,
        },
    )))
}

fn credential_boundary_message(
    request: &SwapRequest<'_>,
    original_provider: &str,
    swapped_provider: &str,
) -> String {
    format!(
        "prompt_id '{}' swaps model '{}' -> '{}', which changes the provider from '{original_provider}' to '{swapped_provider}' after credentials for '{original_provider}' were already resolved. Refusing to send them to '{swapped_provider}'. Point the request at a model whose provider matches the prompt's metadata.model, or set ignore_prompt_manager_model on the prompt to keep the requested model.",
        request.prompt_id.unwrap_or("None"),
        request.original_model,
        request.swapped_model,
    )
}

fn provider_of(resolver: &dyn ModelResolver, model: &str) -> Result<String, ResolutionError> {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    resolver
        .resolve_model(&request)
        .map(|resolved| resolved.provider.slug().to_owned())
}

// LiteLLM's _deployment_model_info_after_prompt_swap: metadata describes the upstream only while the provider holds
#[must_use]
pub fn deployment_model_info_after_prompt_swap(
    requested_provider: Option<&str>,
    resolved_provider: Option<&str>,
    model_info: Option<JsonValue>,
) -> Option<JsonValue> {
    if resolved_provider == requested_provider {
        return model_info;
    }
    None
}
