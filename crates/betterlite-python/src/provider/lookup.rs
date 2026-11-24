use betterlite_models::identity::declared::authenticating_provider;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest, resolve};

use crate::provider::convert::{HandlerRequest, LookupRequest, LookupResult};
use crate::provider::error::ProviderError;
use crate::provider::snapshot::ProviderSnapshot;

// get_llm_provider's own early check raises a ValueError that the outer catch turns into this BadRequestError.
const MODEL_MISSING: &str = "GetLLMProvider Exception - model parameter is required but was None. Please provide a valid model name.\n\noriginal model: None";

pub(crate) fn get_llm_provider(
    snapshot: &ProviderSnapshot,
    request: &LookupRequest,
) -> Result<LookupResult, ProviderError> {
    let Some(model) = request.model.as_deref() else {
        return Err(ProviderError::BadRequest {
            message: MODEL_MISSING.to_owned(),
            model: None,
        });
    };
    let resolved = resolve(
        &ResolutionRequest {
            model,
            custom_llm_provider: request.custom_llm_provider.as_deref(),
            api_base: request.api_base.as_deref(),
            api_key: request.api_key.as_ref(),
        },
        &snapshot.context(),
    )
    .map_err(|error| lookup_error(&error, model))?;
    Ok(LookupResult {
        model: resolved.model,
        provider: resolved.provider.slug().to_owned(),
        api_key: resolved.api_key,
        api_base: resolved.api_base,
    })
}

pub(crate) fn declared_authenticating(request: &HandlerRequest) -> Option<String> {
    authenticating_provider(
        request.model.as_deref(),
        request.custom_llm_provider.as_deref(),
    )
    .map(|provider| <&'static str>::from(provider).to_owned())
}

fn lookup_error(error: &ResolutionError, model: &str) -> ProviderError {
    match error {
        ResolutionError::MissingProvider { model: missing } => ProviderError::BadRequest {
            message: missing_provider_message(missing),
            model: Some(missing.clone()),
        },
        ResolutionError::EndpointWithoutProvider { .. }
        | ResolutionError::DefaultsA { .. }
        | ResolutionError::DefaultsB { .. } => ProviderError::BadRequest {
            message: format!("GetLLMProvider Exception - {error}\n\noriginal model: {model}"),
            model: Some(model.to_owned()),
        },
    }
}

fn missing_provider_message(model: &str) -> String {
    format!(
        "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model={model}\n Pass model as E.g. For 'Huggingface' inference endpoints pass in `completion(model='huggingface/starcoder',..)` Learn more: https://docs.litellm.ai/docs/providers"
    )
}
