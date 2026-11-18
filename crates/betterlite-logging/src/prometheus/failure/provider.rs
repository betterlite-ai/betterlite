use betterlite_error::normalize::class::Class;
use betterlite_error::rate_limit::PROXY_LLM_PROVIDER_FALLBACK;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::resolution::inferred::inferred_provider;
use indexmap::IndexMap;

use super::exception::LabelException;
use super::text_field;

// request_data as the client-side failure path reads it before a deployment is resolved.
#[derive(Clone, Copy)]
pub struct ProviderRequestData<'a> {
    pub litellm_params: Option<&'a IndexMap<String, Json>>,
    pub standard_logging: Option<&'a IndexMap<String, Json>>,
    pub model: Option<&'a str>,
}

// LiteLLM's _extract_api_provider_from_request_data: a resolved provider, then a recovered one, then the model's own.
#[must_use]
pub fn extract_api_provider_from_request_data(
    request: &ProviderRequestData<'_>,
    context: &ResolutionContext<'_>,
) -> Option<String> {
    if let Some(provider) = declared_provider(request.litellm_params, "custom_llm_provider") {
        return Some(provider.to_owned());
    }
    if let Some(provider) = declared_provider(request.standard_logging, "custom_llm_provider") {
        return Some(provider.to_owned());
    }
    let model = request
        .litellm_params
        .and_then(|params| text_field(params, "model"))
        .filter(|model| !model.is_empty())
        .or(request.model)
        .filter(|model| !model.is_empty())?;
    let provider = inferred_provider(model, context)
        .map(|provider| provider.slug().to_owned())
        .filter(|provider| !provider.is_empty());
    if provider.is_none() {
        tracing::debug!(
            model,
            "prometheus: could not infer api_provider from the model"
        );
    }
    provider
}

// litellm_params.custom_llm_provider and its standard_logging_object twin: a falsy value falls through.
fn declared_provider<'a>(fields: Option<&'a IndexMap<String, Json>>, key: &str) -> Option<&'a str> {
    fields
        .and_then(|fields| text_field(fields, key))
        .filter(|value| !value.is_empty())
}

// LiteLLM's _extract_api_provider_from_exception: only a RateLimitError names a provider, and the proxy placeholder stays unlabeled.
#[must_use]
pub fn extract_api_provider_from_exception(exception: &LabelException) -> Option<String> {
    if !matches!(exception.class, Class::RateLimitError) {
        return None;
    }
    exception
        .llm_provider
        .as_deref()
        .filter(|provider| !provider.is_empty() && *provider != PROXY_LLM_PROVIDER_FALLBACK)
        .map(str::to_owned)
}
