use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::ir::DecisionRequest;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::openai_base_config::{Headers, Object as SearchParams};
use betterlite_openai::perplexity_decisions::{DEFAULT_API_BASE, PerplexityDecisionsConfig};
use betterlite_openai::perplexity_responses;
use betterlite_openai::perplexity_search::PerplexitySearch;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::search::provider::{RequestInput, SearchProvider as _, SearchQuery};
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use super::call::{EmbeddingCall, ResourceCall};
use super::error::DispatchError;
use super::logging::convert;

const PROVIDER: &str = "perplexity";

pub mod adapter;
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerplexityRequest {
    pub url: String,
    pub headers: Headers,
    pub body: Vec<u8>,
}

impl PerplexityRequest {
    fn encoded<T>(url: String, headers: Headers, body: &T) -> Result<Self, DispatchError>
    where
        T: Serialize,
    {
        let bytes = serde_json::to_vec(body).map_err(|source| DispatchError::Encode { source })?;
        Ok(Self {
            url,
            headers,
            body: bytes,
        })
    }
}

// main.py's perplexity embedding arm (L7335-L7348): the base and key as they came, empty litellm_params and no headers
pub fn embedding(call: &EmbeddingCall) -> EmbeddingCall {
    let mut resolved = call.clone();
    resolved.litellm_params = Object::new();
    resolved.headers = None;
    resolved
}

// the Perplexity Responses config B5360's tag selects
pub fn responses(
    call: &ResourceCall<'_>,
    input: &ResponsesInput,
    environment: &dyn EnvironmentLookup,
) -> Result<PerplexityRequest, DispatchError> {
    let caller_headers = call.headers.cloned().unwrap_or_default();
    let headers =
        perplexity_responses::validate_environment(caller_headers, call.api_key, environment);
    let body =
        perplexity_responses::transform_request(call.model, input, call.optional_params, None)
            .map_err(|error| DispatchError::PerplexityResponses(Box::new(error)))?;
    let url = perplexity_responses::get_complete_url(call.api_base, environment);
    PerplexityRequest::encoded(url, headers, &body)
}

// the Perplexity search config B5360's tag selects; the key check runs first, as in LiteLLM's search call
pub fn search(
    call: &ResourceCall<'_>,
    query: &SearchQuery,
    environment: &dyn EnvironmentLookup,
) -> Result<PerplexityRequest, DispatchError> {
    let config = PerplexitySearch { environment };
    let optional_params: SearchParams = convert(call.optional_params)?;
    let caller_headers = call.headers.cloned().unwrap_or_default();
    let searched = |error| DispatchError::PerplexitySearch(Box::new(error));
    let headers = config
        .validated_headers(caller_headers.clone(), call.api_key, call.api_base)
        .map_err(searched)?;
    let url = config
        .complete_url(call.api_base, &optional_params, None, call.api_key)
        .map_err(searched)?;
    let body = config
        .transform_search_request(&RequestInput {
            query,
            optional_params: &optional_params,
            api_key: call.api_key,
            api_base: call.api_base,
            headers: &caller_headers,
        })
        .map_err(searched)?;
    PerplexityRequest::encoded(url, headers, &body)
}

fn first_set(
    own: Option<&str>,
    names: &[&str],
    environment: &dyn EnvironmentLookup,
) -> Option<String> {
    own.filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            names
                .iter()
                .find_map(|name| environment.lookup(name).filter(|value| !value.is_empty()))
        })
}

// the Perplexity decisions config B5360's tag selects: the base and key read the config's variable names in order
pub fn decisions(
    call: &ResourceCall<'_>,
    request: &DecisionRequest,
    environment: &dyn EnvironmentLookup,
) -> Result<PerplexityRequest, DispatchError> {
    let config = PerplexityDecisionsConfig;
    let base = first_set(
        call.api_base,
        PerplexityDecisionsConfig::API_BASE_ENV,
        environment,
    )
    .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let own_key = call.api_key.map(ExposeSecret::expose_secret);
    let key = first_set(own_key, PerplexityDecisionsConfig::API_KEY_ENV, environment)
        .map(SecretString::from);
    let caller_headers = call.headers.cloned().unwrap_or_default();
    let body = config
        .transform_decisions_request(call.model, request, PROVIDER)
        .map_err(|error| DispatchError::Decisions(Box::new(error)))?;
    PerplexityRequest::encoded(
        config.get_complete_url(&base, &config.canonical_model(call.model)),
        config.validate_environment(&caller_headers, call.model, key.as_ref()),
        &body,
    )
}
