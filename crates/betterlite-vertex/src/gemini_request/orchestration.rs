use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::Json as ParametersJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use serde::de::DeserializeOwned;

use crate::cache_create::orchestration::{
    CacheCreateOptions, CacheCreateOutcome, CacheCreateRequest, check_and_create_cache,
};
use crate::cache_lookup::target::CacheProvider;
use crate::cache_lookup::transport::CacheTransport;
use crate::model_policy::capabilities::{ModelCapabilityLookup, VertexCapabilityProvider};

use super::body::{BodyInput, Transformed, transform};
use super::error::{BodyError, RequestError, encode};

pub mod async_media;

pub struct CallRequest<'request, F: RemoteMediaFetcher + Sync> {
    pub cache: CacheCreateRequest<'request, F>,
    pub optional_params: &'request Object,
    pub litellm_params: &'request Object,
    pub modify_params: bool,
    pub schema_template: Option<&'request CustomPromptTemplate>,
    pub capabilities: &'request (dyn ModelCapabilityLookup + Sync),
}

pub async fn transform_request_body<T, F, L, C>(
    transport: &T,
    request: CallRequest<'_, F>,
    hooks: &VerboseHooks<L, C>,
) -> Result<Transformed, RequestError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let mut optional_params = request.optional_params.clone();
    let (cached_content, mut options) = cache_options(&mut optional_params)?;
    let cache_request = CacheCreateRequest {
        cached_content: cached_content.as_deref(),
        ..request.cache
    };
    let outcome = check_and_create_cache(transport, &cache_request, &mut options, hooks).await?;
    restore_cache_options(&mut optional_params, &options)?;
    body_from(&cache_request, &request, optional_params, &outcome).await
}

async fn body_from<F>(
    cache: &CacheCreateRequest<'_, F>,
    request: &CallRequest<'_, F>,
    optional_params: Object,
    outcome: &CacheCreateOutcome,
) -> Result<Transformed, RequestError>
where
    F: RemoteMediaFetcher + Sync,
{
    let input = BodyInput {
        provider: capability_provider(cache.provider),
        model: cache.model,
        messages: &outcome.messages,
        optional_params: &optional_params,
        litellm_params: request.litellm_params,
        cached_content: outcome.cached_content.as_deref(),
        modify_params: request.modify_params,
        schema_template: request.schema_template,
    };
    transform(&input, request.capabilities, cache.fetcher)
        .await
        .map_err(RequestError::from)
}

fn cache_options(params: &mut Object) -> Result<(Option<String>, CacheCreateOptions), BodyError> {
    let cached_content = take_typed(params, "cached_content", "a string")?;
    let tools: Option<Vec<ParametersJson>> = take_typed(params, "tools", "a list")?;
    let tool_choice: Option<ParametersJson> = take_typed(params, "tool_choice", "an object")?;
    Ok((cached_content, CacheCreateOptions { tools, tool_choice }))
}

fn take_typed<T>(
    params: &mut Object,
    field: &'static str,
    expected: &'static str,
) -> Result<Option<T>, BodyError>
where
    T: DeserializeOwned,
{
    match params.shift_remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => encode(&value)
            .map(Some)
            .map_err(|_error| BodyError::Field(field, expected)),
    }
}

fn restore_cache_options(
    params: &mut Object,
    options: &CacheCreateOptions,
) -> Result<(), RequestError> {
    if let Some(tools) = &options.tools {
        params.insert("tools".to_owned(), encode(tools)?);
    }
    if let Some(tool_choice) = &options.tool_choice {
        params.insert("tool_choice".to_owned(), encode(tool_choice)?);
    }
    Ok(())
}

const fn capability_provider(provider: CacheProvider) -> VertexCapabilityProvider {
    match provider {
        CacheProvider::Vertex => VertexCapabilityProvider::VertexAi,
        CacheProvider::VertexBeta => VertexCapabilityProvider::VertexAiBeta,
        CacheProvider::Gemini => VertexCapabilityProvider::Gemini,
    }
}
