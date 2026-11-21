use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::api::vector_store_http_write::service::LogContext;

use super::caller::Caller;
use super::dispatch::answer;
use super::error::{GenerateContentFailure, failure};
use super::{GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest};

// setup, dispatch, and the exception_type mapping every one of the four entry points ends with
pub(super) async fn answered<C, S, Executor>(
    deps: &GenerateContentDeps<'_, C, S, Executor>,
    request: &GenerateContentRequest<'_>,
    caller: &Caller,
    log: &mut LogContext<'_, '_>,
) -> Result<GenerateContentAnswer<C::Response>, GenerateContentFailure<C::Error, Executor::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
    Executor: CompletionExecutor + Sync,
{
    answer(deps, request, caller, log).await.map_err(|error| {
        failure(
            error,
            request.model,
            caller.provider.as_deref(),
            deps.redactor,
        )
    })
}
