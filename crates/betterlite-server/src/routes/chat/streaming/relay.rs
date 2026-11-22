use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::HeaderMap;
use betterlite_core::operation::chat::{
    ChatCompletionAnswer, ChatFailure, ChatResult, ChatStreamAnswer, TextCallMode,
};
use betterlite_http::error::HttpError;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::completion_chat_bridge::answer;
use betterlite_openai::stream::SseParser;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_openai::text_completion_execution::{
    CompletionAnswer, CompletionCallAnswer, CompletionStreamAnswer,
};
use betterlite_openai::text_completion_stream::chat_chunk;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_provider::anthropic_dispatch::AnthropicResult;
use betterlite_provider::operation::OperationResult;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use crate::routes::completions::streaming::pump::{FrameSource, Pump, next_frame};

pub(super) struct RelaySource {
    pub(super) upstream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pub(super) parser: SseParser,
    pub(super) model: String,
    pub(super) include_usage: bool,
}

#[async_trait]
impl FrameSource<io::Error> for RelaySource {
    type Item = Bytes;

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
pub(super) struct TextSource {
    pub(super) stream: CompletionStream,
    pub(super) model: String,
    pub(super) include_usage: bool,
}

#[async_trait]
impl FrameSource<HttpError> for TextSource {
    type Item = Bytes;

    async fn fill(&mut self, pending: &mut VecDeque<Bytes>) -> Result<bool, HttpError> {
        text_frame(self, pending).await
    }
}

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
    let Some(value) = chat_chunk(&chunk, &source.model, source.include_usage).map_err(|error| {
        tracing::warn!(message = %error, "the text chunk could not be written");
        HttpError::Timeout
    })?
    else {
        return Ok(false);
    };
    let text = serde_json::to_string(&value).map_err(|error| {
        tracing::warn!(message = %error, "the text chunk could not be written");
        HttpError::Timeout
    })?;
    pending.push_back(Bytes::from(format!("data: {text}\n\n")));
    Ok(false)
}

pub(super) fn relay_bytes(
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
            .is_some_and(|chosen| chosen.include_usage),
    };
    stream::unfold(Pump::new(source), next_frame)
}

pub(super) fn answered_result(
    executor: &ChatExecutor,
    mode: TextCallMode,
    answered: CompletionCallAnswer,
    request: &ChatRequest,
) -> Result<ChatResult, ChatFailure> {
    match (mode, answered) {
        (TextCallMode::Buffered { convert }, CompletionCallAnswer::Buffered(answer)) => {
            let created = i64::try_from((executor.clock)()).unwrap_or(i64::MAX);
            buffered_result(&answer, convert, request.model.as_deref(), created)
        }
        (TextCallMode::Streamed, CompletionCallAnswer::Streamed(answer)) => {
            let include_usage = request
                .stream_options
                .as_ref()
                .is_some_and(|chosen| chosen.include_usage);
            let model = request.model.clone().unwrap_or_default();
            Ok(streamed_result(answer, model, include_usage))
        }
        _ => Err(ChatFailure::Mismatched),
    }
}

fn buffered_result(
    answer: &CompletionAnswer,
    convert: bool,
    asked: Option<&str>,
    created: i64,
) -> Result<ChatResult, ChatFailure> {
    let body = if convert {
        answer::chat_completion_body(&answer.response, created, asked)
    } else {
        answer::text_completion_body(&answer.response, asked)
    }
    .map_err(super::completion_failure)?;
    Ok(ChatResult::Completion(ChatCompletionAnswer {
        body,
        upstream_headers: HeaderMap::new(),
        upstream_time: answer.upstream_time,
    }))
}

pub(super) fn streamed_result(
    answer: CompletionStreamAnswer,
    model: String,
    include_usage: bool,
) -> ChatResult {
    let source = TextSource {
        stream: answer.stream,
        model,
        include_usage,
    };
    ChatResult::Stream(ChatStreamAnswer {
        body: stream::unfold(Pump::new(source), next_frame)
            .map(|item| item.map(|bytes| bytes.to_vec()))
            .boxed(),
        upstream_headers: HeaderMap::new(),
        upstream_time: answer.upstream_time,
    })
}

// the adapter's answer in the shape the call asked for; the call rejects the other shape
pub(super) enum AnthropicAnswer {
    Reply(ChatCompletionAnswer),
    Stream(ChatStreamAnswer),
}

pub(super) fn anthropic_answer(
    result: OperationResult,
    upstream_headers: HeaderMap,
    upstream_time: Duration,
) -> Result<AnthropicAnswer, ChatFailure> {
    let OperationResult::Anthropic(shape) = result else {
        return Err(ChatFailure::Mismatched);
    };
    match shape {
        AnthropicResult::Reply(reply) => {
            let body = serde_json::to_vec(&reply)
                .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
            Ok(AnthropicAnswer::Reply(ChatCompletionAnswer {
                body,
                upstream_headers,
                upstream_time,
            }))
        }
        AnthropicResult::Stream(frames) => Ok(AnthropicAnswer::Stream(ChatStreamAnswer {
            body: frames,
            upstream_headers,
            upstream_time,
        })),
    }
}
