use std::collections::VecDeque;
use std::io;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::response::{IntoResponse as _, Response};
use betterlite_core::message_utilities::response_streams::StreamError;
use betterlite_core::operation::text_stream::TextStreamResult;
use betterlite_http::proxy::StreamResponse;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::text_completion_stream::{DONE_FRAME, without_nulls};
use betterlite_provider::operation::{ChatOperation, OperationRequest, OperationResult};
use betterlite_provider::prepared::deployment::Deployment;
use betterlite_provider::provider::Provider;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use crate::routes::chat::headers::success_headers;
use crate::server::chat_setup::provider_call::{CallError, chat_error};

// The chat endpoint reached for a chat-catalog deployment; routes/chat.rs keeps its streaming module private
pub(crate) async fn provider_stream(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    control: ControlOptions,
) -> Result<StreamResponse, CallError<ChatError>> {
    let provider = Provider::from_deployment(deployment).map_err(chat_error)?;
    let inputs = Deployment::from_chat(deployment);
    let operation = OperationRequest::Stream(Box::new(ChatOperation {
        model: inputs.model.clone(),
        request: request.clone(),
        control,
    }));
    let prepared = provider
        .prepare(&operation, &inputs, &executor.catalog)
        .map_err(chat_error)?;
    let client = executor
        .client
        .deployments
        .get(&deployment.model_id)
        .unwrap_or(&executor.client);
    let answer = provider
        .execute(client, &prepared, &inputs)
        .await
        .map_err(chat_error)?;
    let result = provider.decode(&prepared, answer).map_err(chat_error)?;
    let OperationResult::Stream(stream) = result else {
        return Err(CallError::Mismatched);
    };
    Ok(*stream)
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
    let body = Body::from_stream(frames(answer.body));
    (headers, body).into_response()
}

struct FrameState {
    body: BoxStream<'static, Result<Json, StreamError>>,
    pending: VecDeque<Bytes>,
    finished: bool,
}

// Every wrapper chunk becomes one data event, and the stream closes with the DONE event.
fn frames(
    body: BoxStream<'static, Result<Json, StreamError>>,
) -> impl futures_util::Stream<Item = Result<Bytes, io::Error>> + use<> {
    stream::unfold(
        FrameState {
            body,
            pending: VecDeque::new(),
            finished: false,
        },
        next_frame,
    )
}

async fn next_frame(mut state: FrameState) -> Option<(Result<Bytes, io::Error>, FrameState)> {
    loop {
        if let Some(frame) = state.pending.pop_front() {
            return Some((Ok(frame), state));
        }
        if state.finished {
            return None;
        }
        match state.body.next().await {
            Some(Ok(chunk)) => {
                if let Err(error) = push_chunk(&mut state, &chunk) {
                    state.finished = true;
                    return Some((Err(error), state));
                }
            }
            Some(Err(error)) => {
                state.finished = true;
                return Some((Err(io::Error::other(error)), state));
            }
            None => {
                state.finished = true;
                state
                    .pending
                    .push_back(Bytes::from_static(DONE_FRAME.as_bytes()));
            }
        }
    }
}

fn push_chunk(state: &mut FrameState, chunk: &Json) -> Result<(), io::Error> {
    let value = serde_json::to_value(chunk).map_err(io::Error::other)?;
    let text = serde_json::to_string(without_nulls(value)).map_err(io::Error::other)?;
    state
        .pending
        .push_back(Bytes::from(format!("data: {text}\n\n")));
    Ok(())
}

mod tests;
