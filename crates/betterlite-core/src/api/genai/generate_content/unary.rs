use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::api::vector_store_http_write::service::LogContext;
use crate::genai_setup::logging::mark_async_entrypoint;
use crate::genai_setup::params::GenerateContentLiteLLMParams;

use super::answered::answered;
use super::caller::{ASYNC_ENTRY, Caller, flag_value, moved_config, string_mock};
use super::error::GenerateContentFailure;
use super::fields::field_failure;
use super::mock_generate_content_response;
use super::{GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest};

// generate_content: the marker, generationConfig, the string mock check, then setup and the handler
pub async fn generate_content<C, S, Executor>(
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
    let is_async = flag_value(kwargs.shift_remove(ASYNC_ENTRY).as_ref(), ASYNC_ENTRY)
        .map_err(|error| field_failure(error, request, request.custom_llm_provider, deps))?;
    mark_async_entrypoint(Some(&mut *log.state), ASYNC_ENTRY, is_async);
    let config = moved_config(&mut kwargs, request.config)
        .map_err(|error| field_failure(error, request, request.custom_llm_provider, deps))?;
    let params = GenerateContentLiteLLMParams::from_kwargs(&kwargs)
        .map_err(|error| field_failure(error, request, request.custom_llm_provider, deps))?;
    let caller = Caller {
        model: request.model.to_owned(),
        config,
        kwargs,
        provider: request.custom_llm_provider.map(str::to_owned),
        chat_tools: request.tools.cloned(),
        is_async,
        stream: false,
        mock: string_mock(&params),
    };
    match caller.mock.as_deref() {
        Some(text) => Ok(GenerateContentAnswer::Mock(mock_generate_content_response(
            Some(text),
        ))),
        None => answered(deps, request, &caller, log).await,
    }
}
