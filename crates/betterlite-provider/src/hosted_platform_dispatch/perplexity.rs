use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::ir::DecisionRequest;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::openai_base_config::{Headers, Object as SearchParams};
use betterlite_openai::perplexity_decisions::{DEFAULT_API_BASE, PerplexityDecisionsConfig};
use betterlite_openai::perplexity_embedding;
use betterlite_openai::perplexity_responses;
use betterlite_openai::perplexity_search::PerplexitySearch;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::search::provider::{RequestInput, SearchProvider as _, SearchQuery};
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::call::{EmbeddingCall, ResourceCall};
use super::error::DispatchError;
use super::logging::convert;

const PROVIDER: &str = "perplexity";

mod tests;

// the request a Perplexity config builds: no PreparedRequest variant carries search or decisions yet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerplexityRequest {
    pub url: String,
    pub headers: Headers,
    pub body: Vec<u8>,
}

fn json_body<T>(body: &T) -> Result<Vec<u8>, DispatchError>
where
    T: Serialize,
{
    serde_json::to_vec(body).map_err(|source| DispatchError::Encode { source })
}

// main.py's perplexity embedding arm (L7335-L7348): the base and key as they came, empty litellm_params and no headers
pub fn embedding(call: &EmbeddingCall) -> Result<EmbeddingCall, DispatchError> {
    let mut resolved = call.clone();
    resolved.litellm_params = Object::new();
    resolved.headers = None;
    Ok(resolved)
}

// the shared HTTP embedding handler's request for that call, built by perplexity_embedding
pub fn embedding_request(
    call: &EmbeddingCall,
    environment: &dyn EnvironmentLookup,
) -> Result<PerplexityRequest, DispatchError> {
    let body = perplexity_embedding::transform_embedding_request(
        &call.model,
        &call.input,
        &call.optional_params,
    )
    .map_err(|error| DispatchError::PerplexityEmbedding(Box::new(error)))?;
    Ok(PerplexityRequest {
        url: perplexity_embedding::get_complete_url(call.api_base.as_deref()),
        headers: perplexity_embedding::validate_environment(call.api_key.as_ref(), environment),
        body: json_body(&body)?,
    })
}

// the Perplexity Responses config B5360's tag selects
pub fn responses(
    call: &ResourceCall<'_>,
    input: &ResponsesInput,
    environment: &dyn EnvironmentLookup,
) -> Result<PerplexityRequest, DispatchError> {
    let body =
        perplexity_responses::transform_request(call.model, input, call.optional_params, None)
            .map_err(|error| DispatchError::PerplexityResponses(Box::new(error)))?;
    Ok(PerplexityRequest {
        url: perplexity_responses::get_complete_url(call.api_base, environment),
        headers: perplexity_responses::validate_environment(
            call.headers.cloned().unwrap_or_default(),
            call.api_key,
            environment,
        ),
        body: json_body(&body)?,
    })
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
    Ok(PerplexityRequest {
        url,
        headers,
        body: json_body(&body)?,
    })
}

// the Perplexity decisions config B5360's tag selects: the base and key read the config's variable names in order
pub fn decisions(
    call: &ResourceCall<'_>,
    request: &DecisionRequest,
    environment: &dyn EnvironmentLookup,
) -> Result<PerplexityRequest, DispatchError> {
    let config = PerplexityDecisionsConfig;
    let variable = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| environment.lookup(name).filter(|value| !value.is_empty()))
    };
    let base = call
        .api_base
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| variable(PerplexityDecisionsConfig::API_BASE_ENV))
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let key = call
        .api_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| variable(PerplexityDecisionsConfig::API_KEY_ENV).map(SecretString::from));
    let body = config
        .transform_decisions_request(call.model, request, PROVIDER)
        .map_err(|error| DispatchError::Decisions(Box::new(error)))?;
    Ok(PerplexityRequest {
        url: config.get_complete_url(&base, &config.canonical_model(call.model)),
        headers: config.validate_environment(
            &call.headers.cloned().unwrap_or_default(),
            call.model,
            key.as_ref(),
        ),
        body: json_body(&body)?,
    })
}
