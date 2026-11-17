use std::collections::VecDeque;
use std::io;

use async_trait::async_trait;
use axum::body::Bytes;
use betterlite_http::error::HttpError;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::stream::SseParser;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_openai::text_completion_stream::chat_chunk;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
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
