use std::collections::BTreeMap;
use std::pin::pin;

use betterlite_a2a::agentcore_transform::call::{HandlerCall, SignedRequestCall};
use betterlite_a2a::agentcore_transform::config::{BedrockAgentCoreConfig, ConfigCall};
use betterlite_a2a::agentcore_transform::params::LitellmParams;
use betterlite_a2a::agentcore_transform::runtime::RpcMethod;
use betterlite_a2a::agentcore_transform::{
    AgentCoreError, SignedRequest, TransformError, get_url_and_signed_request, parse_sse_events,
};
use betterlite_a2a::completion_bridge::wire::MessageSendParams;
use betterlite_core::agents::json::AgentJson;
use futures_util::stream;
use futures_util::{Stream, StreamExt as _};
use serde::Serialize;
use tokio::runtime::{Builder, Runtime};

use super::fixture::{Answer, Failure, FixtureRuntime, Source, StreamAnswer};
use crate::compare::helper::HelperError;
use crate::helpers::HelperResult;

fn helper_failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failed(error: &AgentCoreError<Failure>) -> Box<HelperError> {
    let status = match error {
        AgentCoreError::Transform(TransformError::SessionId { .. }) => Some(400),
        AgentCoreError::Transform(_)
        | AgentCoreError::Status { .. }
        | AgentCoreError::ResponseTooLarge { .. }
        | AgentCoreError::Runtime(_) => None,
    };
    Box::new(HelperError {
        class: "AgentCoreError".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn new_runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_failure("RuntimeError", error.to_string()))
}

fn signed_output(request: &SignedRequest) -> HelperResult {
    let body: AgentJson = serde_json::from_slice(&request.body)
        .map_err(|error| helper_failure("SerializationError", error.to_string()))?;
    Ok(serde_json::json!({"url": request.url, "headers": request.headers, "body": body}))
}

async fn drain<S>(events: S) -> Result<Vec<AgentJson>, AgentCoreError<Failure>>
where
    S: Stream<Item = Result<AgentJson, AgentCoreError<Failure>>> + Send,
{
    let mut pinned = pin!(events);
    let mut values = Vec::new();
    while let Some(item) = pinned.next().await {
        values.push(item?);
    }
    Ok(values)
}

#[derive(Clone, Copy)]
pub(super) enum Mode<'case> {
    Signed,
    Buffered(&'case Answer),
    Streamed(&'case StreamAnswer),
}

fn run_signed(runtime: &Runtime, call: &HandlerCall<'_>) -> HelperResult {
    let request = SignedRequestCall {
        request_id: call.request_id,
        params: call.params,
        litellm_params: &call.litellm_params,
        method: RpcMethod::Send,
        agent_extra_headers: call.agent_extra_headers,
    };
    let sent = runtime
        .block_on(get_url_and_signed_request(
            &FixtureRuntime::silent(),
            &request,
            &mut Source,
        ))
        .map_err(|error| failed(&error))?;
    signed_output(&sent)
}

fn run_buffered(runtime: &Runtime, call: &HandlerCall<'_>, answer: &Answer) -> HelperResult {
    let mut handler = FixtureRuntime::buffered(answer).into_handler();
    runtime
        .block_on(handler.handle_non_streaming(call))
        .map_err(|error| failed(&error))
}

fn run_streamed(runtime: &Runtime, call: &HandlerCall<'_>, answer: &StreamAnswer) -> HelperResult {
    let mut handler = FixtureRuntime::streamed(answer).into_handler();
    let events = runtime.block_on(async { drain(handler.handle_streaming(call).await?).await });
    events
        .map(|values| serde_json::json!({"events": values}))
        .map_err(|error| failed(&error))
}

pub(super) fn call(
    request_id: &str,
    params: &MessageSendParams,
    litellm_params: &BTreeMap<String, AgentJson>,
    agent_extra_headers: Option<&BTreeMap<String, String>>,
    mode: Mode<'_>,
) -> HelperResult {
    let parsed = LitellmParams::parse(litellm_params)
        .map_err(|error| helper_failure("TransformError", error.to_string()))?;
    let runtime = new_runtime()?;
    let handler_call = HandlerCall {
        request_id,
        params,
        litellm_params: parsed,
        agent_extra_headers,
    };
    match mode {
        Mode::Signed => run_signed(&runtime, &handler_call),
        Mode::Buffered(answer) => run_buffered(&runtime, &handler_call, answer),
        Mode::Streamed(answer) => run_streamed(&runtime, &handler_call, answer),
    }
}

pub(super) fn sse(lines: &[String]) -> HelperResult {
    let items: Vec<Result<String, Failure>> = lines.iter().cloned().map(Ok).collect();
    let events = new_runtime()?.block_on(drain(parse_sse_events(stream::iter(items))));
    events
        .map(|values| serde_json::json!({"events": values}))
        .map_err(|error| failed(&error))
}

#[derive(Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<AgentJson>,
}

#[derive(Serialize)]
struct ConfigOutput {
    non_streaming: Outcome,
    streaming: Outcome,
}

fn outcome<T>(result: Result<T, AgentCoreError<Failure>>) -> Result<Outcome, Box<HelperError>>
where
    T: Serialize,
{
    match result {
        Ok(value) => Ok(Outcome {
            ok: true,
            value: Some(
                serde_json::to_value(value)
                    .map_err(|error| helper_failure("SerializationError", error.to_string()))?,
            ),
        }),
        Err(AgentCoreError::Transform(TransformError::MissingParams)) => Ok(Outcome {
            ok: false,
            value: None,
        }),
        Err(error) => Err(failed(&error)),
    }
}

pub(super) fn config(
    request_id: &str,
    params: &MessageSendParams,
    litellm_params: Option<&BTreeMap<String, AgentJson>>,
    answer: &Answer,
    stream_answer: &StreamAnswer,
) -> HelperResult {
    let runtime = new_runtime()?;
    let call = ConfigCall {
        request_id,
        params,
        api_base: None,
        litellm_params,
        agent_extra_headers: None,
    };
    let mut buffered = BedrockAgentCoreConfig {
        handler: FixtureRuntime::buffered(answer).into_handler(),
    };
    let mut streamed = BedrockAgentCoreConfig {
        handler: FixtureRuntime::streamed(stream_answer).into_handler(),
    };
    let first = runtime.block_on(buffered.handle_non_streaming(&call));
    let second = runtime.block_on(async { drain(streamed.handle_streaming(&call).await?).await });
    serde_json::to_value(ConfigOutput {
        non_streaming: outcome(first)?,
        streaming: outcome(second)?,
    })
    .map_err(|error| helper_failure("SerializationError", error.to_string()))
}
