use std::collections::BTreeMap;

use betterlite_a2a::a2a_text::{
    A2aResponse, Field, RecordOr, Shape, extract_text_from_a2a_response,
};
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::MAX_TEXT_DEPTH;
use super::error::{A2aError, ErrorDetail, error_message, get_error_class};

const EMPTY_CHUNK: GenericChunk = GenericChunk {
    text: String::new(),
    tool_use: None,
    is_finished: false,
    finish_reason: String::new(),
    usage: None,
    index: Some(0),
    provider_specific_fields: None,
};

#[derive(Deserialize)]
struct StatusState {
    state: Option<Shape>,
}

#[derive(Deserialize)]
struct ResultStatus {
    status: Option<RecordOr<StatusState>>,
}

// result is read here for its state, so the text comes from a second pass over the same bytes
#[derive(Deserialize)]
struct ChunkEnvelope {
    #[serde(default)]
    error: Field<RecordOr<ErrorDetail>>,
    done: Option<Shape>,
    result: Option<RecordOr<ResultStatus>>,
    // flatten makes a chunk that is not a JSON object fail instead of reading as a field list
    #[serde(flatten)]
    _unread: BTreeMap<String, IgnoredAny>,
}

pub fn chunk_parser(chunk: &[u8]) -> Result<GenericChunk, A2aError> {
    let Ok(envelope) = serde_json::from_slice::<ChunkEnvelope>(chunk) else {
        return Ok(EMPTY_CHUNK);
    };
    if let Field::Present(error @ RecordOr::Record(_)) = &envelope.error {
        return Err(get_error_class(&error_message(error), 500, BTreeMap::new()));
    }
    let Ok(response) = serde_json::from_slice::<A2aResponse>(chunk) else {
        return Ok(EMPTY_CHUNK);
    };
    let Ok(text) = extract_text_from_a2a_response(&response, MAX_TEXT_DEPTH) else {
        return Ok(EMPTY_CHUNK);
    };
    let state_finishes = matches!(
        &envelope.result,
        Some(RecordOr::Record(ResultStatus {
            status: Some(RecordOr::Record(StatusState {
                state: Some(Shape::Text(state)),
            })),
        })) if state == "completed" || state == "failed"
    );
    let is_finished = state_finishes || matches!(envelope.done, Some(Shape::Flag(true)));
    Ok(GenericChunk {
        text,
        tool_use: None,
        is_finished,
        finish_reason: if is_finished {
            String::from("stop")
        } else {
            String::new()
        },
        usage: None,
        index: Some(0),
        provider_specific_fields: None,
    })
}
