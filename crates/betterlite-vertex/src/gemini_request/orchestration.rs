use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json as ParametersJson;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::Counter;
use jiff::Timestamp;
use secrecy::SecretString;

use crate::cache_create::orchestration::{
    CacheCreateOptions, CacheCreateOutcome, CacheCreateRequest, check_and_create_cache,
};
use crate::cache_lookup::target::CacheProvider;
use crate::cache_lookup::transport::CacheTransport;
use crate::model_policy::capabilities::{ModelCapabilityLookup, VertexCapabilityProvider};

use super::body::{BodyInput, Transformed, transform};
use super::error::{BodyError, RequestError, encode};
use super::media::reject_inlined_media;

pub struct CallRequest<'request, F: RemoteMediaFetcher + Sync> {
    pub provider: VertexCapabilityProvider,
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request Object,
    pub litellm_params: &'request Object,
    pub modify_params: bool,
    pub schema_template: Option<&'request CustomPromptTemplate>,
    pub api_base: Option<&'request str>,
    pub project: Option<&'request str>,
    pub location: Option<&'request str>,
    pub auth_token: Option<&'request SecretString>,
    pub gemini_api_key: Option<&'request SecretString>,
    pub extra_headers: &'request BTreeMap<String, String>,
    pub deadline: Duration,
    pub disable_transform: bool,
    pub now: Timestamp,
    pub minimum_token_override: Option<u64>,
    pub counter: &'request Counter,
    pub catalog: &'request ModelCatalog,
    pub capabilities: &'request (dyn ModelCapabilityLookup + Sync),
    pub fetcher: &'request F,
}

pub async fn transform_request_body<T, F, L, C>(
    transport: &T,
    request: &CallRequest<'_, F>,
    hooks: &VerboseHooks<L, C>,
) -> Result<Transformed, RequestError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let mut optional_params = request.optional_params.clone();
    let outcome = cache_stage(transport, request, &mut optional_params, hooks).await?;
    body_for(request, &optional_params, &outcome).await
}

// LiteLLM's async entry point downloads remote media before the body; betterlite classifies and refuses instead
pub async fn transform_request_body_async<T, F, L, C>(
    transport: &T,
    request: &CallRequest<'_, F>,
    hooks: &VerboseHooks<L, C>,
) -> Result<Transformed, RequestError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let mut optional_params = request.optional_params.clone();
    let outcome = cache_stage(transport, request, &mut optional_params, hooks).await?;
    reject_inlined_media(request.provider, &outcome.messages)?;
    body_for(request, &optional_params, &outcome).await
}

async fn cache_stage<T, F, L, C>(
    transport: &T,
    request: &CallRequest<'_, F>,
    optional_params: &mut Object,
    hooks: &VerboseHooks<L, C>,
) -> Result<CacheCreateOutcome, RequestError>
where
    T: CacheTransport,
    F: RemoteMediaFetcher + Sync,
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let cached_content = match optional_params.shift_remove("cached_content") {
        None | Some(Json::Null) => None,
        Some(Json::Text(name)) => Some(name),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(RequestError::from(BodyError::Field {
                field: "cached_content",
                expected: "a string",
            }));
        }
    };
    let tools: Option<Vec<ParametersJson>> = match optional_params.shift_remove("tools") {
        None | Some(Json::Null) => None,
        Some(Json::List(items)) => Some(encode(&items)?),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(RequestError::from(BodyError::Field {
                field: "tools",
                expected: "a list",
            }));
        }
    };
    let tool_choice: Option<ParametersJson> = match optional_params.shift_remove("tool_choice") {
        None | Some(Json::Null) => None,
        Some(value) => Some(encode(&value)?),
    };
    let mut options = CacheCreateOptions { tools, tool_choice };
    let outcome = check_and_create_cache(
        transport,
        &CacheCreateRequest {
            provider: cache_provider(request.provider),
            model: request.model,
            messages: request.messages,
            api_base: request.api_base,
            project: request.project,
            location: request.location,
            auth_token: request.auth_token,
            gemini_api_key: request.gemini_api_key,
            extra_headers: request.extra_headers,
            deadline: request.deadline,
            cached_content: cached_content.as_deref(),
            disable_transform: request.disable_transform,
            now: request.now,
            minimum_token_override: request.minimum_token_override,
            counter: request.counter,
            catalog: request.catalog,
            capabilities: request.capabilities,
            fetcher: request.fetcher,
        },
        &mut options,
        hooks,
    )
    .await?;
    // a cache miss without a usable name hands the tools back, and the body still carries them
    if let Some(tools) = options.tools {
        optional_params.insert("tools".to_owned(), encode(&tools)?);
    }
    if let Some(tool_choice) = options.tool_choice {
        optional_params.insert("tool_choice".to_owned(), encode(&tool_choice)?);
    }
    Ok(outcome)
}

async fn body_for<F>(
    request: &CallRequest<'_, F>,
    optional_params: &Object,
    outcome: &CacheCreateOutcome,
) -> Result<Transformed, RequestError>
where
    F: RemoteMediaFetcher + Sync,
{
    let input = BodyInput {
        provider: request.provider,
        model: request.model,
        messages: &outcome.messages,
        optional_params,
        litellm_params: request.litellm_params,
        cached_content: outcome.cached_content.as_deref(),
        modify_params: request.modify_params,
        schema_template: request.schema_template,
    };
    transform(&input, request.capabilities, request.fetcher)
        .await
        .map_err(RequestError::from)
}

const fn cache_provider(provider: VertexCapabilityProvider) -> CacheProvider {
    match provider {
        VertexCapabilityProvider::VertexAi => CacheProvider::Vertex,
        VertexCapabilityProvider::VertexAiBeta => CacheProvider::VertexBeta,
        VertexCapabilityProvider::Gemini => CacheProvider::Gemini,
    }
}
