use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::{MockPlan, resolve_mock};
use betterlite_core::dispatch::chat::mock_stream::mock_frames;
use betterlite_core::mock::chat::{ChatAnswer, answer};
use betterlite_core::mock::stream::StreamSpec;
use betterlite_core::operation::chat::{
    ChatCall, ChatCallInput, ChatCompletionAnswer, ChatFailure, ChatStreamAnswer,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::mock_failure;
use betterlite_openai::chat::{ChatRequest, RequestError, client_mock};
use betterlite_openai::completion_chat_bridge::{
    ChatEndpoint, answer, chat_endpoint, text_endpoint_model, text_request,
};
use betterlite_openai::stream::SseParser;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_openai::text_completion_execution::CompletionExecutor;
use betterlite_openai::text_completion_execution::failure::CompletionFailure;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_stream::chat_chunk;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};
use tokio::sync::OnceCell;

use super::CallContext;
use super::headers::stream_headers;
use crate::routes::completions::streaming::provider_stream;
use crate::server::chat_setup::provider_call::{CallError, answer_chat, control_options};

// the existing chat implementation, reached through the dispatch pipeline's port
pub(super) struct ChatCalls {
    pub(super) executor: Arc<ChatExecutor>,
}

fn chat_failure(error: CallError<ChatError>) -> ChatFailure {
    match error {
        CallError::Route(route) => ChatFailure::Route(route),
        CallError::Operation { source } => ChatFailure::Internal {
            message: source.to_string(),
        },
        CallError::Mismatched => ChatFailure::Mismatched,
    }
}

#[async_trait]
impl ChatCall for ChatCalls {
    fn prepare(&self, request: &ChatRequest) -> Result<ControlOptions, ChatFailure> {
        control_options(request).map_err(ChatFailure::from)
    }

    async fn complete(
        &self,
        call: ChatCallInput<'_>,
        control: ControlOptions,
    ) -> Result<ChatCompletionAnswer, ChatFailure> {
        let (completion, upstream_headers, upstream_time) =
            answer_chat(&self.executor, call.request, &call.deployment, control)
                .await
                .map_err(chat_failure)?;
        let body = serde_json::to_vec(&completion)
            .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
        Ok(ChatCompletionAnswer {
            body,
            upstream_headers,
            upstream_time,
        })
    }

    async fn stream(
        &self,
        call: ChatCallInput<'_>,
        control: ControlOptions,
    ) -> Result<ChatStreamAnswer, ChatFailure> {
        let executor = &self.executor;
        let (client_response, client_tool_calls) =
            client_mock(&call.request.passthrough, executor.mock.permission).map_err(|source| {
                ChatFailure::Route(ChatError::from(RequestError::Body { source }))
            })?;
        let resolved = resolve_mock(
            executor.mock.responses.get(&call.deployment.model_id),
            client_response.as_ref(),
            client_tool_calls.as_deref(),
            call.request.choice_count,
        );
        if let Some(plan) = resolved {
            return mock_stream(executor, &call.deployment, call.request, &plan);
        }
        let started = Instant::now();
        let upstream = provider_stream(executor, &call.deployment, call.request, control)
            .await
            .map_err(chat_failure)?;
        let upstream_time = started.elapsed();
        let upstream_headers = upstream.response.headers().clone();
        Ok(ChatStreamAnswer {
            body: upstream.body_stream(),
            upstream_headers,
            upstream_time,
        })
    }

    // main.py sends a text-completion deployment's chat request through the text endpoint
    fn endpoint(&self, deployment: &ChatDeployment) -> ChatEndpoint {
        chat_endpoint(deployment)
    }

    async fn complete_text(
        &self,
        call: ChatCallInput<'_>,
        _control: ControlOptions,
        convert: bool,
    ) -> Result<ChatCompletionAnswer, ChatFailure> {
        let model = text_endpoint_model(&call.deployment).ok_or(ChatFailure::Mismatched)?;
        let transport =
            text_request(&call.deployment, call.request, &model).map_err(completion_failure)?;
        let executor = text_executor(&self.executor);
        let mut logger = TracingCompletionLog;
        let answered = executor
            .complete_request(
                &call.deployment,
                transport,
                &self.executor.redactor,
                &mut logger,
            )
            .await
            .map_err(completion_failure)?;
        let asked = call.request.model.as_deref();
        let body = if convert {
            let created = i64::try_from((self.executor.clock)()).unwrap_or(i64::MAX);
            answer::chat_completion_body(&answered.response, created, asked)
        } else {
            answer::text_completion_body(&answered.response, asked)
        }
        .map_err(completion_failure)?;
        Ok(ChatCompletionAnswer {
            body,
            // the text answer reaches the chat route without the provider headers a chat answer carries
            upstream_headers: HeaderMap::new(),
            upstream_time: answered.upstream_time,
        })
    }

    async fn stream_text(
        &self,
        call: ChatCallInput<'_>,
        _control: ControlOptions,
    ) -> Result<ChatStreamAnswer, ChatFailure> {
        let model = text_endpoint_model(&call.deployment).ok_or(ChatFailure::Mismatched)?;
        let transport =
            text_request(&call.deployment, call.request, &model).map_err(completion_failure)?;
        let executor = text_executor(&self.executor);
        let mut logger = TracingCompletionLog;
        let answered = executor
            .stream_request(
                &call.deployment,
                transport,
                &self.executor.redactor,
                &mut logger,
            )
            .await
            .map_err(completion_failure)?;
        let include_usage = call
            .request
            .stream_options
            .as_ref()
            .is_some_and(|options| options.include_usage);
        let state = TextBridgeState {
            stream: answered.stream,
            model: call.request.model.clone().unwrap_or_default(),
            include_usage,
            pending: VecDeque::new(),
            finished: false,
        };
        Ok(ChatStreamAnswer {
            body: stream::unfold(state, next_text_frame).boxed(),
            // a streamed text answer carries no provider headers onto the chat route's response
            upstream_headers: HeaderMap::new(),
            upstream_time: answered.upstream_time,
        })
    }
}

fn text_executor(executor: &Arc<ChatExecutor>) -> CompletionExecutor {
    CompletionExecutor {
        client: executor.client.clone(),
        catalog: OnceCell::new(),
    }
}

// the chat route answers with the provider's own error envelope, as LiteLLM's exception path does
fn completion_failure(failure: CompletionFailure) -> ChatFailure {
    let CompletionFailure {
        status,
        error_type,
        param,
        message,
    } = failure;
    let body = serde_json::json!({
        "error": {"message": message, "type": error_type, "param": param}
    });
    match serde_json::to_vec(&body) {
        Ok(body) => ChatFailure::Route(ChatError::Provider {
            response: Box::new(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
        }),
        Err(source) => ChatFailure::Route(ChatError::Encode { source }),
    }
}

// The text provider's chunks rebuilt as chat chunks; B5366's text wrapper is the reverse direction.
struct TextBridgeState {
    stream: CompletionStream,
    model: String,
    include_usage: bool,
    pending: VecDeque<Bytes>,
    finished: bool,
}

async fn next_text_frame(
    mut state: TextBridgeState,
) -> Option<(Result<Bytes, HttpError>, TextBridgeState)> {
    loop {
        if let Some(frame) = state.pending.pop_front() {
            return Some((Ok(frame), state));
        }
        if state.finished {
            return None;
        }
        let Some(item) = state.stream.next().await else {
            state.finished = true;
            state
                .pending
                .push_back(Bytes::from_static(b"data: [DONE]\n\n"));
            continue;
        };
        let chunk = match item {
            Ok(chunk) => chunk,
            Err(error) => {
                // the client reads a broken event stream as a truncated answer, the way LiteLLM's closes
                tracing::warn!(message = %error, "the text provider stream failed");
                state.finished = true;
                return Some((Err(HttpError::Timeout), state));
            }
        };
        match chat_chunk(&chunk, &state.model, state.include_usage) {
            Ok(Some(value)) => match serde_json::to_string(&value) {
                Ok(text) => state
                    .pending
                    .push_back(Bytes::from(format!("data: {text}\n\n"))),
                Err(error) => {
                    tracing::warn!(message = %error, "the text chunk could not be written");
                    state.finished = true;
                    return Some((Err(HttpError::Timeout), state));
                }
            },
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(message = %error, "the text chunk could not be written");
                state.finished = true;
                return Some((Err(HttpError::Timeout), state));
            }
        }
    }
}

pub(super) fn render_stream(
    answer: ChatStreamAnswer,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    call: &CallContext<'_>,
) -> Response {
    let headers = stream_headers(
        &answer.upstream_headers,
        deployment,
        call.id,
        answer.upstream_time,
        call.started.elapsed(),
    );
    let stream_body = Body::from_stream(relay_bytes(answer.body, request));
    (headers, stream_body).into_response()
}

fn mock_stream(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    plan: &MockPlan<'_>,
) -> Result<ChatStreamAnswer, ChatFailure> {
    let answer = answer(plan).map_err(|error| ChatFailure::Route(mock_failure(error)))?;
    let ChatAnswer::Content(content) = answer else {
        // LiteLLM answers a 500 attribute error here; a 400 names the misuse instead
        return Err(ChatFailure::Route(mock_failure(MockError::ObjectStream)));
    };
    let id = format!("chatcmpl-{}", (executor.next_id)());
    let spec = StreamSpec {
        content,
        choice_count: plan.choice_count,
        model: request.model.as_deref().unwrap_or(&deployment.model_group),
        id: &id,
        created: (executor.clock)(),
    };
    let frames = mock_frames(&spec, request, plan.prompt_tokens, executor.mock.usage)
        .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
    let body = stream::iter(frames.into_iter().map(Ok)).boxed();
    Ok(ChatStreamAnswer {
        body,
        upstream_headers: HeaderMap::new(),
        upstream_time: Duration::ZERO,
    })
}

fn relay_bytes(
    upstream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    request: &ChatRequest,
) -> impl futures_util::Stream<Item = Result<Bytes, io::Error>> + use<> {
    let state = RelayState {
        upstream,
        parser: SseParser::default(),
        pending: VecDeque::new(),
        finished: false,
        model: request.model.clone().unwrap_or_default(),
        include_usage: request
            .stream_options
            .as_ref()
            .is_some_and(|options| options.include_usage),
    };
    stream::unfold(state, next_sse)
}

struct RelayState {
    upstream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    parser: SseParser,
    pending: VecDeque<Bytes>,
    finished: bool,
    model: String,
    include_usage: bool,
}

async fn next_sse(mut state: RelayState) -> Option<(Result<Bytes, io::Error>, RelayState)> {
    loop {
        if let Some(event) = state.pending.pop_front() {
            return Some((Ok(event), state));
        }
        if state.finished {
            return None;
        }
        let events = match state.upstream.next().await {
            Some(Ok(chunk)) => state.parser.push(&chunk).map_err(io::Error::other),
            Some(Err(error)) => Err(io::Error::other(error)),
            None => {
                state.finished = true;
                Ok(vec![b"[DONE]".to_vec()])
            }
        };
        if let Err(error) = events.and_then(|parsed| append_events(&mut state, parsed)) {
            state.finished = true;
            return Some((Err(error), state));
        }
    }
}

fn append_events(state: &mut RelayState, events: Vec<Vec<u8>>) -> Result<(), io::Error> {
    for event in events {
        if event == b"[DONE]" {
            state
                .pending
                .push_back(Bytes::from_static(b"data: [DONE]\n\n"));
            state.finished = true;
            break;
        }
        if let Some(chunk) =
            Chunk::parse(&event, &state.model, state.include_usage).map_err(io::Error::other)?
        {
            let text = serde_json::to_string(&chunk).map_err(io::Error::other)?;
            state
                .pending
                .push_back(Bytes::from(format!("data: {text}\n\n")));
        }
    }
    Ok(())
}

mod tests;
