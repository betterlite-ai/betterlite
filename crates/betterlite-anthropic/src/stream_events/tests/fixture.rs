use std::convert::Infallible;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use uuid::Uuid;

use crate::stream_events::StreamEventConverter;
use crate::stream_events::chunk::StreamEventChunk;
use crate::stream_state::state::{IdSource, StreamOptions, StreamState};
use crate::usage::ReasoningTokenCounter;

pub(super) const RESPONSE_UUID: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;

struct FixedIds(Uuid);

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.0
    }
}

pub(super) struct Counter;

impl ReasoningTokenCounter for Counter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

pub(super) fn converter_with(
    json_mode: Option<bool>,
    tool_name_reverse_map: IndexMap<String, String>,
) -> StreamEventConverter<Counter> {
    let ids = FixedIds(Uuid::from_u128(RESPONSE_UUID));
    let state = StreamState::new(
        StreamOptions {
            sync_stream: false,
            json_mode,
            speed: None,
            tool_name_reverse_map,
        },
        &ids,
    );
    StreamEventConverter {
        state,
        counter: Counter,
        json_could_close: false,
    }
}

pub(super) fn new_converter() -> StreamEventConverter<Counter> {
    converter_with(None, IndexMap::new())
}

pub(super) fn reverse_map(rewritten: &str, original: &str) -> IndexMap<String, String> {
    IndexMap::from([(rewritten.to_owned(), original.to_owned())])
}

pub(super) fn parse(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn chunk<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    source: &str,
) -> StreamEventChunk
where
    Counter: ReasoningTokenCounter,
{
    converter.chunk_parser(&parse(source)).unwrap()
}

pub(super) fn at<'value>(value: &'value Json, pointer: &str) -> Option<&'value Json> {
    let mut current = value;
    for segment in pointer.split('/').filter(|segment| !segment.is_empty()) {
        current = match current {
            Json::Object(fields) => fields.get(segment)?,
            Json::List(items) => items.get(segment.parse::<usize>().ok()?)?,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => return None,
        };
    }
    Some(current)
}

pub(super) fn as_json<Value>(value: &Value) -> Json
where
    Value: Serialize + ?Sized,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

pub(super) fn content(chunk: &StreamEventChunk) -> &str {
    &chunk.choices[0].delta.content
}

pub(super) fn finish_reason(chunk: &StreamEventChunk) -> Option<&str> {
    chunk.choices[0].finish_reason.as_deref()
}

pub(super) fn tool_calls(chunk: &StreamEventChunk) -> Option<Json> {
    chunk.choices[0].delta.tool_calls.as_ref().map(as_json)
}

pub(super) fn provider_fields(chunk: &StreamEventChunk) -> Option<Json> {
    chunk.choices[0]
        .delta
        .provider_specific_fields
        .as_ref()
        .map(as_json)
}
