use std::collections::VecDeque;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::HeaderMap;
use betterlite_core::operation::text_completion::TextCompletionFailure;
use betterlite_core::operation::text_stream::TextStreamAnswer;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::StreamResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_stream::decode_chat_events;
use futures_util::stream::{self, BoxStream, StreamExt as _};

use crate::routes::completions::bridge::{ChatChunks, ChunkUsage};

use super::pump::{FrameSource, Pump, next_frame};

pub(in crate::routes::completions) fn frames_answer(
    bytes: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    headers: HeaderMap,
    model: Option<&str>,
    usage: ChunkUsage,
    upstream_time: Duration,
) -> TextStreamAnswer {
    let chunks = ChatChunks {
        bytes,
        parser: SseParser::default(),
        model: model.unwrap_or_default().to_owned(),
        usage,
    };
    let pump: Pump<ChatChunks, Json> = Pump::new(chunks);
    TextStreamAnswer {
        body: stream::unfold(pump, next_frame::<ChatChunks, TextCompletionFailure>).boxed(),
        upstream_headers: headers,
        upstream_time,
    }
}

pub(crate) fn chat_stream_answer(
    upstream: StreamResponse,
    model: Option<&str>,
    include_usage: bool,
    time: Duration,
) -> TextStreamAnswer {
    let headers = upstream.response.headers().clone();
    let body = upstream.body_stream();
    let usage = ChunkUsage::Chat(include_usage);
    frames_answer(body, headers, model, usage, time)
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
        let chunks = match self.usage {
            ChunkUsage::Chat(with_usage) => decode_chat_events(&parsed, &self.model, with_usage)
                .map_err(|error| TextCompletionFailure::internal(&error.to_string()))?,
            ChunkUsage::Anthropic => parsed
                .iter()
                .map(|event| chunk_frame(event, &self.model))
                .collect::<Result<Vec<Json>, TextCompletionFailure>>()?,
        };
        pending.extend(chunks);
        Ok(ended)
    }
}

// The adapter's own chunks are already the text wrapper's chunks: its usage keeps Anthropic's cache breakdown, no [DONE].
fn chunk_frame(event: &[u8], model: &str) -> Result<Json, TextCompletionFailure> {
    let mut frame: Json = serde_json::from_slice(event)
        .map_err(|source| TextCompletionFailure::internal(&source.to_string()))?;
    if let (Json::Object(fields), false) = (&mut frame, model.is_empty()) {
        fields.insert(String::from("model"), Json::Text(model.to_owned()));
    }
    Ok(frame)
}
