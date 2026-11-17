use std::collections::VecDeque;
use std::time::Duration;

use async_trait::async_trait;
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
use crate::routes::completions::streaming::pump::{FrameSource, Pump, next_frame};
use crate::server::chat_setup::provider_call::CallError;

const MISMATCHED: &str = "the provider answered a different operation";

pub(super) async fn text_answer(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
) -> Result<TextCompletionResult, TextCompletionFailure> {
    match text_send(calls, call, false).await? {
        CompletionCallAnswer::Buffered(answer) => Ok(text_result(answer)),
        CompletionCallAnswer::Streamed(_) => Err(TextCompletionFailure::internal(MISMATCHED)),
    }
}

pub(super) fn converted(
    completion: ChatCompletion,
    upstream_headers: HeaderMap,
    upstream_time: Duration,
) -> Result<CompletionAnswer, TextCompletionFailure> {
    let ChatCompletion::OpenAi(response) = completion else {
        return Err(TextCompletionFailure::internal(MISMATCHED));
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
        return Err(TextCompletionFailure::internal(MISMATCHED));
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
                let value = shaped_chunk(chunk, &named)
                    .map_err(|source| TextCompletionFailure::new(500, None, source.to_string()))?;
                serde_json::from_value::<Json>(value)
                    .map_err(|source| TextCompletionFailure::new(500, None, source.to_string()))
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
    let chunks = ChatChunks {
        bytes: upstream.body_stream(),
        parser: SseParser::default(),
        model: model.unwrap_or_default().to_owned(),
        include_usage,
    };
    let pump: Pump<ChatChunks, Json> = Pump::new(chunks);
    TextStreamAnswer {
        body: stream::unfold(pump, next_frame::<ChatChunks, TextCompletionFailure>).boxed(),
        upstream_headers,
        upstream_time,
    }
}

struct ChatChunks {
    bytes: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    parser: SseParser,
    model: String,
    include_usage: bool,
}

#[async_trait]
impl FrameSource<TextCompletionFailure> for ChatChunks {
    type Item = Json;

    async fn fill(&mut self, pending: &mut VecDeque<Json>) -> Result<bool, TextCompletionFailure> {
        let (events, ended) = match self.bytes.next().await {
            Some(Ok(bytes)) => (self.parser.push(&bytes), false),
            Some(Err(error)) => return Err(TextCompletionFailure::internal(&error.to_string())),
            None => (Ok(self.parser.finish()), true),
        };
        let parsed =
            events.map_err(|source| TextCompletionFailure::internal(&source.to_string()))?;
        let chunks = decode_chat_events(&parsed, &self.model, self.include_usage)
            .map_err(|error| TextCompletionFailure::internal(&error.to_string()))?;
        pending.extend(chunks);
        Ok(ended)
    }
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
        CallError::Operation { source } => TextCompletionFailure::internal(&source.to_string()),
        CallError::Mismatched => TextCompletionFailure::internal(MISMATCHED),
    }
}
