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
    ChatCall, ChatCallInput, ChatCompletionAnswer, ChatFailure, ChatResult, ChatStreamAnswer,
    TextCallMode,
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
use betterlite_openai::text_completion_execution::failure::CompletionFailure;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_execution::{CompletionCallAnswer, CompletionExecutor};
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

    // The text endpoint's answer in the shape the route asked for; no provider headers reach the client.
    async fn text(
        &self,
        call: ChatCallInput<'_>,
        mode: TextCallMode,
    ) -> Result<ChatResult, ChatFailure> {
        match mode {
            // converted to chat unless the deployment is a fine-tuned model
            TextCallMode::Buffered { convert } => {
                let answered = bridged_send(self, &call, false).await?;
                let CompletionCallAnswer::Buffered(answered) = answered else {
                    return Err(ChatFailure::Mismatched);
                };
                let asked = call.request.model.as_deref();
                let body = if convert {
                    let created = i64::try_from((self.executor.clock)()).unwrap_or(i64::MAX);
                    answer::chat_completion_body(&answered.response, created, asked)
                } else {
                    answer::text_completion_body(&answered.response, asked)
                }
                .map_err(completion_failure)?;
                Ok(ChatResult::Completion(ChatCompletionAnswer {
                    body,
                    upstream_headers: HeaderMap::new(),
                    upstream_time: answered.upstream_time,
                }))
            }
            // rewritten as the chat route's SSE events
            TextCallMode::Streamed => {
                let answered = bridged_send(self, &call, true).await?;
                let CompletionCallAnswer::Streamed(answered) = answered else {
                    return Err(ChatFailure::Mismatched);
                };
                let source = TextSource {
                    stream: answered.stream,
                    model: call.request.model.clone().unwrap_or_default(),
                    include_usage: call
                        .request
                        .stream_options
                        .as_ref()
                        .is_some_and(|options| options.include_usage),
                };
                Ok(ChatResult::Stream(ChatStreamAnswer {
                    body: stream::unfold(Pump::new(source), next_frame)
                        .map(|item| item.map(|bytes| bytes.to_vec()))
                        .boxed(),
                    upstream_headers: HeaderMap::new(),
                    upstream_time: answered.upstream_time,
                }))
            }
        }
    }
}

// The text endpoint reached for this chat deployment: the built request sent the way the route asked
async fn bridged_send(
    calls: &ChatCalls,
    call: &ChatCallInput<'_>,
    streamed: bool,
) -> Result<CompletionCallAnswer, ChatFailure> {
    let model = text_endpoint_model(&call.deployment).ok_or(ChatFailure::Mismatched)?;
    let transport =
        text_request(&call.deployment, call.request, &model).map_err(completion_failure)?;
    let mut logger = TracingCompletionLog;
    text_executor(&calls.executor)
        .send(
            &call.deployment,
            transport,
            streamed,
            &calls.executor.redactor,
            &mut logger,
        )
        .await
        .map_err(completion_failure)
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

// A source the frame pump drains: fills the queue and answers true once it has ended.
#[async_trait]
trait FrameSource<Error: Send + 'static>: Send {
    async fn fill(&mut self, pending: &mut VecDeque<Bytes>) -> Result<bool, Error>;
}

struct Pump<Source> {
    source: Source,
    pending: VecDeque<Bytes>,
    finished: bool,
}

impl<Source> Pump<Source> {
    fn new(source: Source) -> Self {
        Self {
            source,
            pending: VecDeque::new(),
            finished: false,
        }
    }
}

// One data event at a time, then the DONE event; the source's error closes the stream.
async fn next_frame<Source, Error>(
    mut state: Pump<Source>,
) -> Option<(Result<Bytes, Error>, Pump<Source>)>
where
    Source: FrameSource<Error> + Send,
{
    loop {
        if let Some(frame) = state.pending.pop_front() {
            return Some((Ok(frame), state));
        }
        if state.finished {
            return None;
        }
        match state.source.fill(&mut state.pending).await {
            Ok(false) => {}
            Ok(true) => state.finished = true,
            Err(error) => {
                state.finished = true;
                return Some((Err(error), state));
            }
        }
    }
}

// The provider's SSE bytes relayed as the chat route's events, model restamped and usage filtered.
struct RelaySource {
    upstream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    parser: SseParser,
    model: String,
    include_usage: bool,
}

#[async_trait]
impl FrameSource<io::Error> for RelaySource {
    async fn fill(&mut self, pending: &mut VecDeque<Bytes>) -> Result<bool, io::Error> {
        let events = match self.upstream.next().await {
            Some(Ok(chunk)) => self.parser.push(&chunk).map_err(io::Error::other)?,
            Some(Err(error)) => return Err(io::Error::other(error)),
            None => {
                pending.push_back(Bytes::from_static(b"data: [DONE]\n\n"));
                return Ok(true);
            }
        };
        let mut ended = false;
        for event in events {
            if event == b"[DONE]" {
                pending.push_back(Bytes::from_static(b"data: [DONE]\n\n"));
                ended = true;
                break;
            }
            if let Some(chunk) =
                Chunk::parse(&event, &self.model, self.include_usage).map_err(io::Error::other)?
            {
                let text = serde_json::to_string(&chunk).map_err(io::Error::other)?;
                pending.push_back(Bytes::from(format!("data: {text}\n\n")));
            }
        }
        Ok(ended)
    }
}

// The text provider's chunks rebuilt as chat chunks; B5366's text wrapper is the reverse direction.
struct TextSource {
    stream: CompletionStream,
    model: String,
    include_usage: bool,
}

#[async_trait]
impl FrameSource<HttpError> for TextSource {
    async fn fill(&mut self, pending: &mut VecDeque<Bytes>) -> Result<bool, HttpError> {
        text_frame(self, pending).await
    }
}

// One text chunk becomes at most one chat event; DONE closes the stream after the provider's last chunk.
async fn text_frame(
    source: &mut TextSource,
    pending: &mut VecDeque<Bytes>,
) -> Result<bool, HttpError> {
    let Some(item) = source.stream.next().await else {
        pending.push_back(Bytes::from_static(b"data: [DONE]\n\n"));
        return Ok(true);
    };
    let chunk = item.map_err(|error| {
        // the client reads a broken event stream as a truncated answer, the way LiteLLM's closes
        tracing::warn!(message = %error, "the text provider stream failed");
        HttpError::Timeout
    })?;
    match chat_chunk(&chunk, &source.model, source.include_usage) {
        Ok(Some(value)) => {
            let text = serde_json::to_string(&value).map_err(|error| {
                tracing::warn!(message = %error, "the text chunk could not be written");
                HttpError::Timeout
            })?;
            pending.push_back(Bytes::from(format!("data: {text}\n\n")));
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(message = %error, "the text chunk could not be written");
            return Err(HttpError::Timeout);
        }
    }
    Ok(false)
}

fn relay_bytes(
    upstream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    request: &ChatRequest,
) -> impl futures_util::Stream<Item = Result<Bytes, io::Error>> + use<> {
    let source = RelaySource {
        upstream,
        parser: SseParser::default(),
        model: request.model.clone().unwrap_or_default(),
        include_usage: request
            .stream_options
            .as_ref()
            .is_some_and(|options| options.include_usage),
    };
    stream::unfold(Pump::new(source), next_frame)
}

mod tests;
