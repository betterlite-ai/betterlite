use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, resolve,
};
use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::genai_setup::params::{GenerateContentLiteLLMParams, RequestFieldError};

use super::error::{GenerateContentError, GenerateContentFailure, failure};
use super::{GenerateContentDeps, GenerateContentRequest};

// CallTypes.agenerate_content and CallTypes.agenerate_content_stream, the keys _mark_async_entrypoint writes
pub(super) const ASYNC_ENTRY: &str = "agenerate_content";
pub(super) const STREAM_ENTRY: &str = "agenerate_content_stream";

// one entry point's own steps, and every value its dispatch reads back
pub(super) struct Caller {
    pub(super) model: String,
    pub(super) config: Option<Object>,
    pub(super) kwargs: Object,
    pub(super) provider: Option<String>,
    pub(super) chat_tools: Option<Json>,
    pub(super) is_async: bool,
    pub(super) stream: bool,
    pub(super) mock: Option<String>,
}

// kwargs.pop(marker, False): a present marker that is not a boolean is rejected by name
pub(super) fn pop_flag(
    kwargs: &mut Object,
    marker: &'static str,
) -> Result<bool, RequestFieldError> {
    match kwargs.shift_remove(marker) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(flag)) => Ok(flag),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(RequestFieldError {
                field: marker,
                expected: "a boolean",
            })
        }
    }
}

// kwargs.pop("generationConfig") when no config was passed: LiteLLM moves it into the config argument
pub(super) fn moved_config(
    kwargs: &mut Object,
    config: Option<&Object>,
) -> Result<Option<Object>, RequestFieldError> {
    if let Some(given) = config {
        return Ok(Some(given.clone()));
    }
    match kwargs.shift_remove("generationConfig") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(RequestFieldError {
                field: "generationConfig",
                expected: "an object",
            })
        }
    }
}

// compatibility.md: an explicit "" is a string used as given, so it answers the mock too
pub(super) fn string_mock(params: &GenerateContentLiteLLMParams) -> Option<String> {
    params
        .mock_response
        .as_ref()
        .and_then(Json::as_str)
        .map(str::to_owned)
}

// get_llm_provider(model=model, custom_llm_provider=...): the async entry points resolve before they map errors
pub(super) fn entry_provider<C, S, Executor>(
    deps: &GenerateContentDeps<'_, C, S, Executor>,
    request: &GenerateContentRequest<'_>,
) -> Result<Option<String>, GenerateContentFailure<C::Error, Executor::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
    Executor: CompletionExecutor,
{
    request.custom_llm_provider.map_or_else(
        || {
            resolved(deps.resolution, request.model)
                .map(Some)
                .map_err(|error| {
                    failure(
                        GenerateContentError::Resolve(error),
                        request.model,
                        None,
                        deps.redactor,
                    )
                })
        },
        |given| Ok(Some(given.to_owned())),
    )
}

// get_llm_provider(model=model): the async entry points pass no provider, api_base or api_key
fn resolved(context: &ResolutionContext<'_>, model: &str) -> Result<String, ResolutionError> {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    resolve(&request, context).map(|chosen| chosen.provider.slug().to_owned())
}
