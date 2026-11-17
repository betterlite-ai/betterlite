use std::collections::VecDeque;
use std::time::Duration;

use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::operation::text_completion::{
    TextCompletionCallInput, TextCompletionFailure, TextCompletionResult,
};
use betterlite_core::operation::text_stream::TextStreamAnswer;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::StreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::completion_chat_bridge::answer::text_completion_response;
use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_execution::failure::CompletionFailure;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_execution::{
    CompletionAnswer, CompletionCallAnswer, CompletionSend, CompletionStreamAnswer,
};
use betterlite_openai::text_completion_stream::{decode_chat_events, shaped_chunk};
use betterlite_provider::operation::ChatCompletion;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use super::CompletionCalls;
use crate::server::chat_setup::provider_call::CallError;

const MISMATCHED: &str = "the provider answered a different operation";

pub(super) async fn text_answer(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
) -> Result<TextCompletionResult, TextCompletionFailure> {
    match text_send(calls, call, false).await? {
        CompletionCallAnswer::Buffered(answer) => Ok(text_result(answer)),
        CompletionCallAnswer::Streamed(_) => Err(internal_failure(MISMATCHED)),
    }
}

pub(super) fn converted(
    completion: ChatCompletion,
    upstream_headers: HeaderMap,
    upstream_time: Duration,
) -> Result<CompletionAnswer, TextCompletionFailure> {
    let ChatCompletion::OpenAi(response) = completion else {
        return Err(internal_failure(MISMATCHED));
    };
    Ok(CompletionAnswer {
        response: Box::new(text_completion_response(*response)),
        upstream_headers,
        upstream_time,
    })
}

pub(super) async fn text_stream(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
) -> Result<TextStreamAnswer, TextCompletionFailure> {
    let CompletionCallAnswer::Streamed(answer) = text_send(calls, call, true).await? else {
        return Err(internal_failure(MISMATCHED));
    };
    let model = call.request.model.as_deref();
    Ok(stream_answer(answer, model))
}

async fn text_send(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
    streamed: bool,
) -> Result<CompletionCallAnswer, TextCompletionFailure> {
    let mut logger = TracingCompletionLog;
    let built = calls
        .execution
        .text_transport(&call.deployment, call.request, streamed)
        .await
        .map_err(bridge_failure)?;
    calls
        .execution
        .send(CompletionSend {
            deployment: &call.deployment,
            request: built,
            streamed,
            redactor: &calls.state.executor.redactor,
            logger: &mut logger,
        })
        .await
        .map_err(bridge_failure)
}

pub(super) fn text_result(answer: CompletionAnswer) -> TextCompletionResult {
    TextCompletionResult {
        response: answer.response,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

fn stream_answer(answer: CompletionStreamAnswer, model: Option<&str>) -> TextStreamAnswer {
    let named = model.unwrap_or_default().to_owned();
    let body = answer
        .stream
        .map(move |item| match item {
            Ok(chunk) => {
                let value =
                    shaped_chunk(chunk, &named).map_err(|source| stream_failure(&source))?;
                serde_json::from_value::<Json>(value).map_err(|source| stream_failure(&source))
            }
            Err(error) => Err(TextCompletionFailure::new(500, None, error.to_string())),
        })
        .boxed();
    TextStreamAnswer {
        body,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

pub(super) fn chat_stream_answer(
    upstream: StreamResponse,
    model: Option<&str>,
    include_usage: bool,
    upstream_time: Duration,
) -> TextStreamAnswer {
    let upstream_headers = upstream.response.headers().clone();
    let state = ChatChunkState {
        bytes: upstream.body_stream(),
        parser: SseParser::default(),
        model: model.unwrap_or_default().to_owned(),
        include_usage,
        pending: VecDeque::new(),
        finished: false,
    };
    TextStreamAnswer {
        body: stream::unfold(state, next_chat_chunk).boxed(),
        upstream_headers,
        upstream_time,
    }
}

struct ChatChunkState {
    bytes: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    parser: SseParser,
    model: String,
    include_usage: bool,
    pending: VecDeque<Json>,
    finished: bool,
}

async fn next_chat_chunk(
    mut state: ChatChunkState,
) -> Option<(Result<Json, TextCompletionFailure>, ChatChunkState)> {
    loop {
        if let Some(chunk) = state.pending.pop_front() {
            return Some((Ok(chunk), state));
        }
        if state.finished {
            return None;
        }
        match state.bytes.next().await {
            Some(Ok(bytes)) => {
                let parsed = state
                    .parser
                    .push(&bytes)
                    .map_err(|source| internal_failure(&source.to_string()))
                    .and_then(|events| {
                        decode_chat_events(&events, &state.model, state.include_usage)
                            .map_err(|error| internal_failure(&error.to_string()))
                    });
                if let Err(error) = enqueue(&mut state, parsed) {
                    state.finished = true;
                    return Some((Err(error), state));
                }
            }
            Some(Err(error)) => {
                state.finished = true;
                return Some((Err(internal_failure(&error.to_string())), state));
            }
            None => {
                state.finished = true;
                let decoded =
                    decode_chat_events(&state.parser.finish(), &state.model, state.include_usage)
                        .map_err(|error| internal_failure(&error.to_string()));
                if let Err(error) = enqueue(&mut state, decoded) {
                    return Some((Err(error), state));
                }
            }
        }
    }
}

fn enqueue(
    state: &mut ChatChunkState,
    parsed: Result<Vec<Json>, TextCompletionFailure>,
) -> Result<(), TextCompletionFailure> {
    for chunk in parsed? {
        state.pending.push_back(chunk);
    }
    Ok(())
}

// the chat endpoint's text answer is written by the bridge, so the route adds its own content type
pub(super) fn json_response(mut headers: HeaderMap, body: Vec<u8>) -> Response {
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    (headers, body).into_response()
}

pub(super) fn bridge_failure(failure: CompletionFailure) -> TextCompletionFailure {
    TextCompletionFailure {
        status: failure.status,
        error_type: failure.error_type,
        param: failure.param,
        message: failure.message,
    }
}

fn stream_failure(source: &serde_json::Error) -> TextCompletionFailure {
    TextCompletionFailure::new(500, None, source.to_string())
}

fn internal_failure(message: &str) -> TextCompletionFailure {
    TextCompletionFailure::new(500, Some("internal_server_error"), message.to_owned())
}

pub(super) fn chat_call_failure(
    error: CallError<ChatError>,
    calls: &CompletionCalls,
) -> TextCompletionFailure {
    match error {
        CallError::Route(route) => {
            let failure = route.failure(&calls.state.executor.redactor);
            TextCompletionFailure {
                status: failure.status,
                error_type: Some(failure.error_type),
                param: failure.param,
                message: route.to_string(),
            }
        }
        CallError::Operation { source } => internal_failure(&source.to_string()),
        CallError::Mismatched => internal_failure(MISMATCHED),
    }
}
