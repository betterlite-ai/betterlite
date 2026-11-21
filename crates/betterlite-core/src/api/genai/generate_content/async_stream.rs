use betterlite_prompts::arguments::json::Json;

use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::api::vector_store_http_write::service::LogContext;
use crate::genai_setup::logging::mark_async_entrypoint;

use super::answered::answered;
use super::caller::{Caller, STREAM_ENTRY, entry_provider, moved_config};
use super::error::GenerateContentFailure;
use super::fields::field_failure;
use super::{GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest};

// agenerate_content_stream: mark the stream entry, move generationConfig, resolve the provider, then stream
pub async fn agenerate_content_stream<C, S, Executor>(
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
    kwargs.insert(STREAM_ENTRY.to_owned(), Json::Bool(true));
    mark_async_entrypoint(Some(&mut *log.state), STREAM_ENTRY, true);
    let config = moved_config(&mut kwargs, request.config)
        .map_err(|error| field_failure(error, request, request.custom_llm_provider, deps))?;
    let provider = entry_provider(deps, request)?;
    let caller = Caller {
        model: request.model.to_owned(),
        config,
        kwargs,
        provider,
        chat_tools: request.tools.cloned(),
        is_async: true,
        stream: true,
        mock: None,
    };
    answered(deps, request, &caller, log).await
}
