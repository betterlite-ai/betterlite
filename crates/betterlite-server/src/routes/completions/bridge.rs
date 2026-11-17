use std::collections::VecDeque;
use std::time::Duration;

use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use betterlite_core::operation::text_completion::{
    TextCompletionCallInput, TextCompletionFailure, TextCompletionResult,
};
use betterlite_core::operation::text_stream::TextStreamAnswer;
use betterlite_http::proxy::StreamResponse;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::completion_chat_bridge::answer::text_completion_response;
use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_execution::failure::CompletionFailure;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_execution::{
    CompletionAnswer, CompletionCallAnswer, CompletionStreamAnswer,
};
use betterlite_openai::text_completion_stream::{decode_chat_events, shaped_chunk};
use betterlite_provider::operation::ChatCompletion;
use futures_util::StreamExt as _;

use super::CompletionCalls;
use crate::server::chat_setup::provider_call::CallError;

pub(super) async fn text_answer(
    calls: &CompletionCalls,
    call: TextCompletionCallInput<'_>,
) -> Result<TextCompletionResult, TextCompletionFailure> {
    let CompletionCallAnswer::Buffered(answer) = text_send(calls, &call, false).await? else {
        return Err(internal_failure(
            "the provider answered a different operation",
        ));
    };
    Ok(text_result(answer))
}

// the chat answer as the text completion response the route writes
pub(super) fn converted(
    completion: ChatCompletion,
    upstream_headers: HeaderMap,
    upstream_time: Duration,
) -> Result<CompletionAnswer, TextCompletionFailure> {
    let ChatCompletion::OpenAi(response) = completion else {
        return Err(internal_failure(
            "the provider answered a different operation",
        ));
    };
    Ok(CompletionAnswer {
        response: Box::new(text_completion_response(*response)),
        upstream_headers,
        upstream_time,
    })
}

pub(super) async fn text_stream(
    calls: &CompletionCalls,
    call: TextCompletionCallInput<'_>,
) -> Result<TextStreamAnswer, TextCompletionFailure> {
    streamed_answer(text_send(calls, &call, true).await)
        .map(|answer| stream_answer(answer, call.request.model.as_deref()))
}

// The text endpoint reached for this deployment, with the stream flag the dropped body loses
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
        .send(
            &call.deployment,
            built,
            streamed,
            &calls.state.executor.redactor,
            &mut logger,
        )
        .await
        .map_err(bridge_failure)
}

fn streamed_answer(
    answered: Result<CompletionCallAnswer, TextCompletionFailure>,
) -> Result<CompletionStreamAnswer, TextCompletionFailure> {
    match answered? {
        CompletionCallAnswer::Streamed(answer) => Ok(answer),
        CompletionCallAnswer::Buffered(_) => Err(internal_failure(
            "the provider answered a different operation",
        )),
    }
}

// the provider's buffered answer as the dispatch pipeline's text result
pub(super) fn text_result(answer: CompletionAnswer) -> TextCompletionResult {
    TextCompletionResult {
        response: answer.response,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

// The text endpoint's chunks as the dispatch pipeline's stream answer, model restamped.
fn stream_answer(answer: CompletionStreamAnswer, model: Option<&str>) -> TextStreamAnswer {
    let model = model.unwrap_or_default().to_owned();
    let body = answer
        .stream
        .map(move |item| match item {
            Ok(chunk) => shaped_chunk(chunk, &model)
                .map_err(stream_failure)
                .and_then(|value| {
                    serde_json::from_value::<betterlite_models::decisions::Json>(value)
                        .map_err(stream_failure)
                }),
            Err(error) => Err(TextCompletionFailure {
                status: 500,
                error_type: None,
                param: None,
                message: error.to_string(),
            }),
        })
        .boxed();
    TextStreamAnswer {
        body,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

// The chat endpoint's bytes as the dispatch pipeline's stream answer the wrapper converts.
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
        body: futures_util::stream::unfold(state, next_chat_chunk).boxed(),
        upstream_headers,
        upstream_time,
    }
}

// The chat endpoint's SSE bytes read as the chat chunks the text wrapper converts, model restamped.
struct ChatChunkState {
    bytes: futures_util::stream::BoxStream<
        'static,
        Result<Vec<u8>, betterlite_http::error::HttpError>,
    >,
    parser: SseParser,
    model: String,
    include_usage: bool,
    pending: VecDeque<betterlite_models::decisions::Json>,
    finished: bool,
}

async fn next_chat_chunk(
    mut state: ChatChunkState,
) -> Option<(
    Result<betterlite_models::decisions::Json, TextCompletionFailure>,
    ChatChunkState,
)> {
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
    parsed: Result<Vec<betterlite_models::decisions::Json>, TextCompletionFailure>,
) -> Result<(), TextCompletionFailure> {
    for chunk in parsed? {
        state.pending.push_back(chunk);
    }
    Ok(())
}

// the chat endpoint's text answer is written by the bridge, so the route adds its own content type
pub(super) fn json_response(mut headers: HeaderMap, body: Vec<u8>) -> Response {
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
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

fn stream_failure(source: serde_json::Error) -> TextCompletionFailure {
    TextCompletionFailure {
        status: 500,
        error_type: None,
        param: None,
        message: source.to_string(),
    }
}

fn internal_failure(message: &str) -> TextCompletionFailure {
    TextCompletionFailure {
        status: 500,
        error_type: Some(String::from("internal_server_error")),
        param: None,
        message: message.to_owned(),
    }
}

// the chat route's status, type and param for a failure the text provider answered with
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
        CallError::Mismatched => internal_failure("the provider answered a different operation"),
    }
}
