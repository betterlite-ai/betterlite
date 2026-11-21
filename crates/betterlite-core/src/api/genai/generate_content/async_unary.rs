use betterlite_prompts::arguments::json::Json;

use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::api::vector_store_http_write::service::LogContext;

use super::caller::{ASYNC_ENTRY, entry_provider, moved_config};
use super::error::GenerateContentFailure;
use super::fields::field_failure;
use super::unary::generate_content;
use super::{GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest};

// agenerate_content: set the marker, move generationConfig, resolve the provider, then run the sync entry point
pub async fn agenerate_content<C, S, Executor>(
    deps: &GenerateContentDeps<'_, C, S, Executor>,
    request: &GenerateContentRequest<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<GenerateContentAnswer<C::Response>, GenerateContentFailure<C::Error, Executor::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
    Executor: CompletionExecutor + Sync,
{
    let mut kwargs = request.kwargs.clone();
    kwargs.insert(ASYNC_ENTRY.to_owned(), Json::Bool(true));
    let config = moved_config(&mut kwargs, request.config)
        .map_err(|error| field_failure(error, request, request.custom_llm_provider, deps))?;
    let provider = entry_provider(deps, request)?;
    let marked = GenerateContentRequest {
        model: request.model,
        contents: request.contents,
        config: config.as_ref(),
        tools: request.tools,
        extra_headers: request.extra_headers,
        extra_query: request.extra_query,
        extra_body: request.extra_body,
        timeout: request.timeout,
        custom_llm_provider: provider.as_deref(),
        kwargs: &kwargs,
    };
    generate_content(deps, &marked, log).await
}
