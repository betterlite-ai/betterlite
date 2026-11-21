use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::api::vector_store_http_write::service::LogContext;
use crate::genai_setup::logging::mark_async_entrypoint;

use super::answered::answered;
use super::caller::{Caller, STREAM_ENTRY, moved_config, pop_flag};
use super::error::GenerateContentFailure;
use super::fields::field_failure;
use super::{GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest};

// generate_content_stream: the stream marker, generationConfig, and the adapter that receives no tools
pub async fn generate_content_stream<C, S, Executor>(
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
    let is_async = pop_flag(&mut kwargs, STREAM_ENTRY)
        .map_err(|error| field_failure(error, request, request.custom_llm_provider, deps))?;
    mark_async_entrypoint(Some(&mut *log.state), STREAM_ENTRY, is_async);
    let config = moved_config(&mut kwargs, request.config)
        .map_err(|error| field_failure(error, request, request.custom_llm_provider, deps))?;
    let caller = Caller {
        model: request.model.to_owned(),
        config,
        kwargs,
        provider: request.custom_llm_provider.map(str::to_owned),
        chat_tools: None,
        is_async,
        stream: true,
        mock: None,
    };
    answered(deps, request, &caller, log).await
}
