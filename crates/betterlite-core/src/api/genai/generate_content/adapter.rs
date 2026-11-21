use crate::api::genai::chat_call::{
    CompletionExecutor, GenerateContentCall as AdapterCall, Handled, generate_content_handler,
};
use crate::api::genai_chat_input::TranslationContext;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::genai_setup::setup::GenerateContentSetupResult;

use super::adapter_input::{adapter_input, adapter_params};
use super::caller::Caller;
use super::error::GenerateContentError;
use super::{GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest};

// GenerateContentToCompletionHandler.generate_content_handler, or its await when _is_async is set
pub(super) async fn adapter<C, S, Executor>(
    deps: &GenerateContentDeps<'_, C, S, Executor>,
    request: &GenerateContentRequest<'_>,
    caller: &Caller,
    setup: &GenerateContentSetupResult<'_, '_, C>,
) -> Result<GenerateContentAnswer<C::Response>, GenerateContentError<C::Error, Executor::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
    Executor: CompletionExecutor + Sync,
{
    let input = adapter_input(request, caller, &setup.config_dict)?;
    let params = adapter_params(&setup.litellm_params);
    let context = TranslationContext {
        resolution: deps.resolution,
        model_capabilities: deps.capabilities,
        limits: deps.limits,
    };
    let call = AdapterCall {
        request: &input.request,
        litellm_params: &params,
        context: &context,
        stream: caller.stream,
        forwarded: input.forwarded,
    };
    let handled = generate_content_handler(deps.executor, call, caller.is_async)?;
    match handled {
        Handled::Done(outcome) => Ok(GenerateContentAnswer::Adapter(outcome)),
        Handled::Pending(pending) => pending
            .await
            .map(GenerateContentAnswer::Adapter)
            .map_err(GenerateContentError::Adapter),
    }
}
