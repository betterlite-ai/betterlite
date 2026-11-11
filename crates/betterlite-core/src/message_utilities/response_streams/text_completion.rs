use std::pin::Pin;
use std::task::{Context, Poll};

use betterlite_models::decisions::Json;
use futures_util::Stream;
use indexmap::IndexMap;

use super::StreamError;
use crate::message_utilities::json;
use crate::message_utilities::small_helpers::EMPTY_USAGE;

pub struct TextCompletionStreamWrapper<S> {
    pub chunks: S,
    pub stream_options: Option<Json>,
}

impl<S: Iterator<Item = Json>> Iterator for TextCompletionStreamWrapper<S> {
    type Item = Result<Json, StreamError>;

    fn next(&mut self) -> Option<Self::Item> {
        let chunk = self.chunks.next()?;
        if matches!(chunk, Json::Null) || json::text(&chunk) == Some("None") {
            return Some(Err(StreamError::MissingChunk));
        }
        Some(convert_chunk(
            &chunk,
            include_usage(self.stream_options.as_ref()),
        ))
    }
}

impl<S: Stream<Item = Json> + Unpin> Stream for TextCompletionStreamWrapper<S> {
    type Item = Result<Json, StreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match Pin::new(&mut this.chunks).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(chunk)) => {
                if matches!(chunk, Json::Null) || json::text(&chunk) == Some("None") {
                    return Poll::Ready(Some(Err(StreamError::MissingChunk)));
                }
                let include_usage = include_usage(this.stream_options.as_ref());
                Poll::Ready(Some(convert_chunk(&chunk, include_usage)))
            }
        }
    }
}

// `stream_options and stream_options.get(...) is True` accepts only a JSON true, not a truthy value
fn include_usage(stream_options: Option<&Json>) -> bool {
    stream_options
        .and_then(json::object)
        .and_then(|options| options.get("include_usage"))
        .is_some_and(|value| matches!(value, Json::Bool(true)))
}

fn convert_chunk(chunk: &Json, include_usage: bool) -> Result<Json, StreamError> {
    let fields = json::object(chunk).ok_or(StreamError::NotAnObject)?;
    let choice = text_choice(chunk)?;
    let mut response = IndexMap::new();
    response.insert(
        String::from("id"),
        fields.get("id").cloned().unwrap_or(Json::Null),
    );
    response.insert(
        String::from("object"),
        Json::Text(String::from("text_completion")),
    );
    response.insert(
        String::from("created"),
        fields.get("created").cloned().unwrap_or(Json::Null),
    );
    response.insert(
        String::from("model"),
        fields.get("model").cloned().unwrap_or(Json::Null),
    );
    response.insert(String::from("choices"), Json::List(vec![choice]));
    let usage = if include_usage {
        fields.get("usage").cloned().unwrap_or(Json::Null)
    } else {
        EMPTY_USAGE.to_json()
    };
    response.insert(String::from("usage"), usage);
    Ok(Json::Object(response))
}

fn text_choice(chunk: &Json) -> Result<Json, StreamError> {
    let fields = json::object(chunk).ok_or(StreamError::NotAnObject)?;
    let Some(Json::List(choices)) = fields.get("choices") else {
        return Err(StreamError::NoChoices);
    };
    let choice = choices
        .first()
        .and_then(json::object)
        .ok_or(StreamError::NoChoices)?;
    let delta = choice
        .get("delta")
        .and_then(json::object)
        .ok_or(StreamError::NoDelta)?;
    // ModelResponseStream gives an absent delta content, index and finish_reason None, 0 and None
    let text = delta.get("content").cloned().unwrap_or(Json::Null);
    let reasoning = delta
        .get("reasoning_content")
        .cloned()
        .unwrap_or(Json::Null);
    let index = choice
        .get("index")
        .cloned()
        .unwrap_or_else(|| Json::Number(serde_json::Number::from(0_u8)));
    let finish_reason = choice.get("finish_reason").cloned().unwrap_or(Json::Null);
    let mut text_choice = IndexMap::new();
    text_choice.insert(String::from("finish_reason"), finish_reason);
    text_choice.insert(String::from("index"), index);
    text_choice.insert(String::from("text"), text);
    text_choice.insert(String::from("logprobs"), Json::Null);
    text_choice.insert(String::from("reasoning_content"), reasoning);
    Ok(Json::Object(text_choice))
}
