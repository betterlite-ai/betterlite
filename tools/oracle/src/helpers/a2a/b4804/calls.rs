use std::collections::BTreeMap;

use betterlite_a2a::agentcore_transform::call::{AgentCoreHandler, HandlerCall, SignedRequestCall};
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

use super::fixture::{Answer, Failure, FixtureRuntime, Source, StreamAnswer};
use crate::compare::helper::HelperError;
use crate::helpers::HelperResult;

pub(super) fn helper_failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn failed(error: &AgentCoreError<Failure>) -> Box<HelperError> {
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

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_failure("RuntimeError", error.to_string()))
}

fn signed_output(request: &SignedRequest) -> HelperResult {
    let body = serde_json::from_slice(&request.body)
        .map_err(|error| helper_failure("SerializationError", error.to_string()))?;
    Ok(serde_json::json!({"url": request.url, "headers": request.headers, "body": body}))
}

fn handler_call<'call>(
    request_id: &'call str,
    params: &'call MessageSendParams,
    litellm_params: LitellmParams,
    agent_extra_headers: Option<&'call BTreeMap<String, String>>,
) -> HandlerCall<'call> {
    HandlerCall {
        request_id,
        params,
        litellm_params,
        agent_extra_headers,
    }
}

pub(super) enum Mode<'case> {
    Signed,
    Buffered(&'case Answer),
    Streamed(&'case StreamAnswer),
}

pub(super) fn call(
    request_id: &str,
    params: &MessageSendParams,
    litellm_params: &BTreeMap<String, AgentJson>,
    agent_extra_headers: Option<&BTreeMap<String, String>>,
    mode: Mode<'_>,
) -> HelperResult {
    let parameters = LitellmParams::parse(litellm_params)
        .map_err(|error| helper_failure("TransformError", error.to_string()))?;
    let runtime = runtime()?;
    match mode {
        Mode::Signed => {
            let request = runtime.block_on(get_url_and_signed_request(
                &FixtureRuntime {
                    answer: None,
                    stream: None,
                },
                &SignedRequestCall {
                    request_id,
                    params,
                    litellm_params: &parameters,
                    method: RpcMethod::Send,
                    agent_extra_headers,
                },
                &mut Source,
            ));
            request.map_or_else(
                |error| Err(failed(&error)),
                |request| signed_output(&request),
            )
        }
        Mode::Buffered(answer) => {
            let mut handler = AgentCoreHandler {
                runtime: FixtureRuntime {
                    answer: Some(answer),
                    stream: None,
                },
                ids: Source,
            };
            let handler_call = handler_call(request_id, params, parameters, agent_extra_headers);
            match runtime.block_on(handler.handle_non_streaming(&handler_call)) {
                Ok(value) => Ok(value),
                Err(error) => Err(failed(&error)),
            }
        }
        Mode::Streamed(answer) => {
            let mut handler = AgentCoreHandler {
                runtime: FixtureRuntime {
                    answer: None,
                    stream: Some(answer),
                },
                ids: Source,
            };
            let handler_call = handler_call(request_id, params, parameters, agent_extra_headers);
            match runtime.block_on(handler.handle_streaming(&handler_call)) {
                Ok(events) => Ok(serde_json::json!({"events": collect(&runtime, events)?})),
                Err(error) => Err(failed(&error)),
            }
        }
    }
}

pub(super) fn sse(lines: &[String]) -> HelperResult {
    let items: Vec<Result<String, Failure>> = lines.iter().cloned().map(Ok).collect();
    let result = runtime()?.block_on(async {
        let mut events = std::pin::pin!(parse_sse_events(stream::iter(items.into_iter())));
        let mut values = Vec::new();
        while let Some(item) = events.next().await {
            match item {
                Ok(value) => values.push(value),
                Err(error) => return Err(error),
            }
        }
        Ok(values)
    });
    match result {
        Ok(values) => Ok(serde_json::json!({"events": values})),
        Err(error) => Err(failed(&error)),
    }
}

pub(super) fn collect<S>(
    runtime: &tokio::runtime::Runtime,
    stream: S,
) -> Result<Vec<AgentJson>, Box<HelperError>>
where
    S: Stream<Item = Result<AgentJson, AgentCoreError<Failure>>>,
{
    let values = runtime.block_on(async {
        let mut stream = std::pin::pin!(stream);
        let mut values = Vec::new();
        while let Some(item) = stream.next().await {
            match item {
                Ok(value) => values.push(value),
                Err(error) => return Err(error),
            }
        }
        Ok(values)
    });
    values.map_err(|error| failed(&error))
}

#[derive(serde::Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<AgentJson>,
}

#[derive(serde::Serialize)]
struct ConfigOutput {
    non_streaming: Outcome,
    streaming: Outcome,
}

pub(super) fn config(
    request_id: &str,
    params: &MessageSendParams,
    litellm_params: Option<&BTreeMap<String, AgentJson>>,
    answer: &Answer,
    stream_answer: &StreamAnswer,
) -> HelperResult {
    let runtime = runtime()?;
    let mut non_streaming = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime: FixtureRuntime {
                answer: Some(answer),
                stream: None,
            },
            ids: Source,
        },
    };
    let call = ConfigCall {
        request_id,
        params,
        api_base: None,
        litellm_params,
        agent_extra_headers: None,
    };
    let first = match runtime.block_on(non_streaming.handle_non_streaming(&call)) {
        Ok(value) => Outcome {
            ok: true,
            value: Some(value),
        },
        Err(AgentCoreError::Transform(TransformError::MissingParams)) => Outcome {
            ok: false,
            value: None,
        },
        Err(error) => return Err(failed(&error)),
    };
    let mut streaming = BedrockAgentCoreConfig {
        handler: AgentCoreHandler {
            runtime: FixtureRuntime {
                answer: None,
                stream: Some(stream_answer),
            },
            ids: Source,
        },
    };
    let second = match runtime.block_on(streaming.handle_streaming(&call)) {
        Ok(events) => Outcome {
            ok: true,
            value: Some(serde_json::json!(collect(&runtime, events)?)),
        },
        Err(AgentCoreError::Transform(TransformError::MissingParams)) => Outcome {
            ok: false,
            value: None,
        },
        Err(error) => return Err(failed(&error)),
    };
    serde_json::to_value(ConfigOutput {
        non_streaming: first,
        streaming: second,
    })
    .map_err(|error| helper_failure("SerializationError", error.to_string()))
}
