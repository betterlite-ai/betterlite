use betterlite_logging::logging_pre_post::env::PrePostEnv;

use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::api::vector_store_http_write::service::LogContext;
use crate::genai_setup::setup::{SetupRequest, setup_generate_content_call};

use super::adapter::adapter;
use super::caller::Caller;
use super::error::GenerateContentError;
use super::native::{NativeInput, native};
use super::{GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest};

// setup once, then the provider config decides between the native handler and the chat adapter
pub(super) async fn answer<C, S, Executor>(
    deps: &GenerateContentDeps<'_, C, S, Executor>,
    request: &GenerateContentRequest<'_>,
    caller: &Caller,
    log: &mut LogContext<'_, '_>,
) -> Result<GenerateContentAnswer<C::Response>, GenerateContentError<C::Error, Executor::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
    Executor: CompletionExecutor + Sync,
{
    let env: &PrePostEnv<'_> = log.env;
    let setup = setup_generate_content_call(
        &SetupRequest {
            model: &caller.model,
            contents: request.contents,
            config: caller.config.as_ref(),
            custom_llm_provider: caller.provider.as_deref(),
            tools: request.tools,
            kwargs: &caller.kwargs,
        },
        deps.configs,
        Some(&mut *log.state),
        deps.resolution,
    )?;
    let Some(config) = setup.provider_config else {
        return adapter(deps, request, caller, &setup).await;
    };
    native(
        deps,
        config,
        NativeInput {
            request,
            caller,
            setup,
        },
        env,
    )
    .await
}
