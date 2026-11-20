use super::PromptError;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::error::CredentialBoundaryError;
use crate::api::responses_provider_selection::resolution::ModelResolver;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest};
use thiserror::Error;
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
pub fn resolve_swapped_provider(
    request: &SwapRequest<'_>,
    kwargs: &ParameterMap,
    resolver: &dyn ModelResolver,
) -> Result<String, PromptError> {
    let swapped_provider = match provider_of(resolver, request.swapped_model) {
        Ok(provider) => provider,
        Err(error) => return Err(PromptError::Swap(PromptSwapError::Resolution(error))),
    };
    let key_absent = kwargs
        .get("api_key")
        .filter(|value| **value != JsonValue::Null)
        .is_none();
    let base_absent = kwargs
        .get("api_base")
        .filter(|value| **value != JsonValue::Null)
        .is_none();
    if key_absent && base_absent {
        return Ok(swapped_provider);
    }
    let original_provider = match request.custom_llm_provider {
        Some(provider) => provider.to_owned(),
        None => match provider_of(resolver, request.original_model) {
            Ok(provider) => provider,
            Err(ResolutionError::MissingProvider { .. }) => return Ok(swapped_provider),
            Err(error) => return Err(PromptError::Swap(PromptSwapError::Resolution(error))),
        },
    };
    if swapped_provider == original_provider {
        return Ok(swapped_provider);
    }
    Err(PromptError::Swap(PromptSwapError::CredentialBoundary(
        CredentialBoundaryError {
            message: format!(
                "prompt_id '{}' swaps model '{}' -> '{}', which changes the provider from '{original_provider}' to '{swapped_provider}' after credentials for '{original_provider}' were already resolved. Refusing to send them to '{swapped_provider}'. Point the request at a model whose provider matches the prompt's metadata.model, or set ignore_prompt_manager_model on the prompt to keep the requested model.",
                request.prompt_id.unwrap_or("None"),
                request.original_model,
                request.swapped_model,
            ),
            model: request.swapped_model.to_owned(),
            provider: swapped_provider,
        },
    )))
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
#[must_use]
pub fn model_info_after_swap(
    requested_provider: Option<&str>,
    resolved_provider: Option<&str>,
    model_info: Option<JsonValue>,
) -> Option<JsonValue> {
    if resolved_provider == requested_provider {
        return model_info;
    }
    None
}
