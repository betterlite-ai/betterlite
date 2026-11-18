use betterlite_error::normalize::class::Class;
use betterlite_error::rate_limit::PROXY_LLM_PROVIDER_FALLBACK;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::resolution::inferred::inferred_provider;
use indexmap::IndexMap;

use super::exception::LabelException;
use super::text_field;

#[derive(Clone, Copy)]
pub struct ProviderRequestData<'request> {
    pub litellm_params: Option<&'request IndexMap<String, Json>>,
    pub standard_logging: Option<&'request IndexMap<String, Json>>,
    pub model: Option<&'request str>,
}

#[must_use]
pub fn api_provider_from_request_data(
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
fn declared_provider<'fields>(
    fields: Option<&'fields IndexMap<String, Json>>,
    key: &str,
) -> Option<&'fields str> {
    fields
        .and_then(|declared| text_field(declared, key))
        .filter(|value| !value.is_empty())
}

#[must_use]
pub fn api_provider_from_exception(exception: &LabelException) -> Option<String> {
    if !matches!(exception.class, Class::RateLimitError) {
        return None;
    }
    exception
        .llm_provider
        .as_deref()
        .filter(|provider| !provider.is_empty() && *provider != PROXY_LLM_PROVIDER_FALLBACK)
        .map(str::to_owned)
}
