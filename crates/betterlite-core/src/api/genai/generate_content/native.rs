use std::sync::Arc;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_logging::logging_pre_post::env::PrePostEnv;

use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::error::GenAiHttpError;
use crate::api::genai_http::service::{
    ClientSource, GenerateContentCall as HttpCall, GenerateContentDeps as HttpDeps, StreamSettings,
    async_generate_content as async_native_call, generate_content as native_call,
};
use crate::api::vector_store_http_write::service::{LogContext, SuppliedClient};
use crate::genai_setup::merge::merge_native_request_fields;
use crate::genai_setup::setup::GenerateContentSetupResult;

use super::caller::Caller;
use super::error::{GenerateContentError, TimeoutError};
use super::fields::first_non_empty;
use super::{GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest};

// what the native handler reads off the setup result and the entry point's own records
pub(super) struct NativeInput<'call, 'request, C> {
    pub(super) request: &'call GenerateContentRequest<'request>,
    pub(super) caller: &'call Caller,
    pub(super) setup: GenerateContentSetupResult<'call, 'call, C>,
}

// base_llm_http_handler.generate_content_handler: the body, the merged native fields and the timeout
pub(super) async fn native<C, S, Executor>(
    deps: &GenerateContentDeps<'_, C, S, Executor>,
    config: &C,
    input: NativeInput<'_, '_, C>,
    env: &PrePostEnv<'_>,
) -> Result<GenerateContentAnswer<C::Response>, GenerateContentError<C::Error, Executor::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
    Executor: CompletionExecutor,
{
    let NativeInput {
        request,
        caller,
        setup,
    } = input;
    let GenerateContentSetupResult {
        model,
        custom_llm_provider,
        config_dict,
        native_request_fields,
        litellm_params,
        logging,
        ..
    } = setup;
    let timeout = resolved_timeout(request.timeout, deps.request_timeout)?;
    let extra_body = merge_native_request_fields(&native_request_fields, request.extra_body);
    let instruction = first_non_empty(&caller.kwargs, ["systemInstruction", "system_instruction"]);
    let call = HttpCall {
        model: &model,
        contents: request.contents,
        config: &config_dict,
        tools: request.tools,
        custom_llm_provider: &custom_llm_provider,
        litellm_params: &litellm_params,
        extra_headers: request.extra_headers,
        extra_body: extra_body.as_ref(),
        system_instruction: instruction,
        timeout: Some(timeout),
        stream: caller.stream,
    };
    sent(
        deps,
        config,
        &call,
        LogContext {
            state: logging,
            env,
        },
        caller.is_async,
    )
    .await
    .map_err(GenerateContentError::Native)
}

// one POST through the handler the entry's kind selects
async fn sent<C, S, Executor>(
    deps: &GenerateContentDeps<'_, C, S, Executor>,
    config: &C,
    call: &HttpCall<'_>,
    mut log: LogContext<'_, '_>,
    is_async: bool,
) -> Result<GenerateContentAnswer<C::Response>, GenAiHttpError<C::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    let http = http_deps(deps, config);
    if is_async {
        return async_native_call(&http, call, &mut log)
            .await
            .map(GenerateContentAnswer::AsyncNative);
    }
    native_call(&http, call, &mut log)
        .await
        .map(GenerateContentAnswer::Native)
}

// the provider config and the client pool the handler runs against, as B3488's own deps record
fn http_deps<'http, C, S, Executor>(
    deps: &'http GenerateContentDeps<'_, C, S, Executor>,
    config: &'http C,
) -> HttpDeps<'http, C, S>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
{
    HttpDeps {
        config,
        source: deps.source,
        supplied: deps
            .supplied_client
            .as_ref()
            .map(|supplied| SuppliedClient {
                kind: supplied.kind,
                transport: supplied.transport,
            }),
        stream: StreamSettings {
            line_limit: deps.stream.line_limit,
            clock: Arc::clone(&deps.stream.clock),
            logging: deps.stream.logging.clone(),
        },
    }
}

// timeout or request_timeout: a zero timeout is no timeout, so the configured REQUEST_TIMEOUT stands in
fn resolved_timeout(
    given: Option<Duration>,
    request_timeout: RequestTimeout,
) -> Result<Duration, TimeoutError> {
    if let Some(timeout) = given
        && !timeout.is_zero()
    {
        return Ok(timeout);
    }
    Duration::try_from_secs_f64(request_timeout.seconds).map_err(|source| TimeoutError {
        seconds: request_timeout.seconds,
        source,
    })
}
