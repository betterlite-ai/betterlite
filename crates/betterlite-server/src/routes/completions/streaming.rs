use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::response::{IntoResponse as _, Response};
use betterlite_core::message_utilities::response_streams::StreamError;
use betterlite_core::operation::text_completion::{TextCompletionCallInput, TextCompletionFailure};
use betterlite_core::operation::text_stream::{TextStreamAnswer, TextStreamResult};
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_execution::CompletionAnswer;
use betterlite_openai::text_completion_request::CompletionRequest;
use betterlite_openai::text_completion_stream::{DONE_FRAME, decode_chat_events, without_nulls};
use betterlite_provider::anthropic_dispatch::gateway::runtime::AnthropicRuntime;
use betterlite_provider::operation::{
    ChatCompletion, ChatOperation, OperationError, OperationRequest, OperationResult,
};
use betterlite_provider::prepared::PreparedRequest;
use betterlite_provider::prepared::deployment::Deployment;
use betterlite_provider::provider::Provider;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use crate::routes::chat::headers::success_headers;
use crate::routes::chat::streaming::recovery;
use crate::server::chat_setup::provider_call::{CallError, chat_error};

use self::pump::{FrameSource, Pump, next_frame};

use super::CompletionCalls;
use super::bridge::{chat_call_failure, chat_stream_answer, converted};

// The route whose upstream rules a chat stream follows: only the chat route's provider adds stream usage
pub(crate) enum StreamUsage {
    Chat,
    Text,
}

// The chat endpoint reached for a chat-catalog deployment; routes/chat.rs keeps its streaming module private
pub(crate) async fn provider_stream(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    control: ControlOptions,
    usage: StreamUsage,
    anthropic: Option<&Arc<AnthropicRuntime>>,
) -> Result<(OperationResult, HeaderMap), CallError<ChatError>> {
    let provider = Provider::from_deployment(deployment).map_err(chat_error)?;
    let mut inputs = Deployment::from_chat(deployment);
    inputs.anthropic = anthropic.cloned();
    let operation = OperationRequest::Stream(Box::new(ChatOperation {
        model: inputs.model.clone(),
        request: request.clone(),
        control,
    }));
    let mut prepared = provider
        .prepare(&operation, &inputs, &executor.catalog)
        .map_err(chat_error)?;
    // atext_completion never asks the chat provider for stream usage unless the caller chose it
    if matches!(usage, StreamUsage::Text) && request.stream_options.is_none() {
        strip_stream_usage(&mut prepared)?;
    }
    let client = executor
        .client
        .deployments
        .get(&deployment.model_id)
        .unwrap_or(&executor.client);
    let answer = provider
        .execute(client, &prepared, &inputs)
        .await
        .map_err(chat_error)?;
    let upstream_headers = answer.headers().clone();
    let result = provider.decode(&prepared, answer).map_err(chat_error)?;
    Ok((result, upstream_headers))
}

// the chat route's auto-injected usage options have no place in a bridged completion's provider body
fn strip_stream_usage(prepared: &mut PreparedRequest) -> Result<(), CallError<ChatError>> {
    let PreparedRequest::Stream(chat) = prepared else {
        return Ok(());
    };
    let Json::Object(mut fields) = serde_json::from_slice(&chat.http.body)
        .map_err(|source| chat_error(OperationError::Encode { source }))?
    else {
        return Ok(());
    };
    if fields.shift_remove("stream_options").is_some() {
        chat.http.body = serde_json::to_vec(&Json::Object(fields))
            .map_err(|source| chat_error(OperationError::Encode { source }))?;
    }
    Ok(())
}

// The completion route's stream headers: the text answer is an event stream the client reads at once
pub(super) fn render(
    answer: TextStreamResult,
    deployment: &ChatDeployment,
    call_id: &str,
    started: Instant,
) -> Response {
    let mut headers = success_headers(
        &HeaderMap::new(),
        deployment,
        call_id,
        answer.upstream_time,
        started.elapsed(),
    );
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    let chunks = ChunkFrames { body: answer.body };
    let pump = stream::unfold(Pump::new(chunks), next_frame::<_, io::Error>);
    let body = Body::from_stream(pump);
    (headers, body).into_response()
}

struct ChunkFrames {
    body: BoxStream<'static, Result<Json, StreamError>>,
}

#[async_trait]
impl FrameSource<io::Error> for ChunkFrames {
    type Item = Bytes;

    async fn fill(&mut self, pending: &mut VecDeque<Bytes>) -> Result<bool, io::Error> {
        let Some(item) = self.body.next().await else {
            pending.push_back(Bytes::from_static(DONE_FRAME.as_bytes()));
            return Ok(true);
        };
        let chunk = item.map_err(io::Error::other)?;
        let value = serde_json::to_value(&chunk).map_err(io::Error::other)?;
        let text = serde_json::to_string(&without_nulls(value)).map_err(io::Error::other)?;
        pending.push_back(Bytes::from(format!("data: {text}\n\n")));
        Ok(false)
    }
}

// the completion route's buffered chat answer: the provider's completion, or the output-limit 400's reply, converted
pub(super) async fn buffered(
    calls: &CompletionCalls,
    request: &CompletionRequest,
    answered: (OperationResult, HeaderMap, Duration),
) -> Result<CompletionAnswer, TextCompletionFailure> {
    let (result, headers, time) = answered;
    let (completion, recovered) = if let OperationResult::OutputLimit { sent } = &result {
        recovered_completion(calls, sent, headers, request.model.as_deref()).await?
    } else {
        let OperationResult::Chat(completion) = result else {
            return Err(chat_call_failure(CallError::Mismatched, calls));
        };
        (completion, headers)
    };
    converted(completion, recovered, time)
}

async fn recovered_completion(
    calls: &CompletionCalls,
    sent: &JsonValue,
    headers: HeaderMap,
    model: Option<&str>,
) -> Result<(ChatCompletion, HeaderMap), TextCompletionFailure> {
    let Some(counter) = &calls.counter else {
        return Err(chat_call_failure(CallError::Mismatched, calls));
    };
    recovery::recovered_completion(&calls.state.executor, counter, sent, headers, model)
        .await
        .map_err(|failure| TextCompletionFailure::internal(&failure.to_string()))
}

// the same answer streamed: the provider's chat stream, or the two recovered frames, read as the route's chunks
pub(super) async fn streamed(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
    chat: &ChatRequest,
    control: ControlOptions,
) -> Result<TextStreamAnswer, TextCompletionFailure> {
    let started = Instant::now();
    let (result, headers) = provider_stream(
        &calls.state.executor,
        &call.deployment,
        chat,
        control,
        StreamUsage::Text,
        None,
    )
    .await
    .map_err(|error| chat_call_failure(error, calls))?;
    if let OperationResult::OutputLimit { sent } = &result {
        return recovered_stream(calls, call.request, sent, headers, started.elapsed()).await;
    }
    let OperationResult::Stream(stream) = result else {
        return Err(chat_call_failure(CallError::Mismatched, calls));
    };
    let usage = include_usage(call.request);
    Ok(chat_stream_answer(
        *stream,
        call.request.model.as_deref(),
        usage,
        started.elapsed(),
    ))
}

async fn recovered_stream(
    calls: &CompletionCalls,
    request: &CompletionRequest,
    sent: &JsonValue,
    headers: HeaderMap,
    upstream_time: Duration,
) -> Result<TextStreamAnswer, TextCompletionFailure> {
    let Some(counter) = &calls.counter else {
        return Err(chat_call_failure(CallError::Mismatched, calls));
    };
    let (frames, recovered) =
        recovery::recovered_frames(&calls.state.executor, counter, sent, headers)
            .await
            .map_err(|failure| TextCompletionFailure::internal(&failure.to_string()))?;
    let mut parser = SseParser::default();
    let mut events = Vec::new();
    for frame in frames {
        let parsed = parser
            .push(&frame)
            .map_err(|source| TextCompletionFailure::internal(&source.to_string()))?;
        events.extend(parsed);
    }
    events.extend(parser.finish());
    let model = request.model.as_deref().unwrap_or_default();
    let chunks = decode_chat_events(&events, model, include_usage(request))
        .map_err(|source| TextCompletionFailure::internal(&source.to_string()))?;
    Ok(TextStreamAnswer {
        body: stream::iter(chunks.into_iter().map(Ok)).boxed(),
        upstream_headers: recovered,
        upstream_time,
    })
}

fn include_usage(request: &CompletionRequest) -> bool {
    request
        .stream_options
        .as_ref()
        .is_some_and(|chosen| chosen.include_usage)
}

pub(crate) mod pump;
mod tests;
