use std::collections::BTreeMap;

use betterlite_a2a::a2a_text::{A2aResponse, extract_text_from_a2a_response};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use super::MAX_TEXT_DEPTH;
use super::error::{A2aError, get_error_class};

const EMPTY_CHUNK: GenericChunk = GenericChunk {
    text: String::new(),
    tool_use: None,
    is_finished: false,
    finish_reason: String::new(),
    usage: None,
    index: Some(0),
    provider_specific_fields: None,
};

fn part_text(chunk: &Json) -> Option<String> {
    let shape = serde_json::to_value(chunk).ok()?;
    let response: A2aResponse = serde_json::from_value(shape).ok()?;
    extract_text_from_a2a_response(&response, MAX_TEXT_DEPTH).ok()
}

fn finish_reason(fields: &Object) -> Option<&'static str> {
    if let Some(Json::Object(result)) = fields.get("result")
        && let Some(Json::Object(status)) = result.get("status")
        && let Some(Json::Text(state)) = status.get("state")
        && (state == "completed" || state == "failed")
    {
        return Some("stop");
    }
    if let Some(Json::Bool(true)) = fields.get("done") {
        return Some("stop");
    }
    None
}

fn build_chunk(chunk: &Json, fields: &Object) -> GenericChunk {
    let Some(text) = part_text(chunk) else {
        return EMPTY_CHUNK;
    };
    let reason = finish_reason(fields);
    GenericChunk {
        text,
        tool_use: None,
        is_finished: reason.is_some(),
        finish_reason: reason.unwrap_or_default().to_owned(),
        usage: None,
        index: Some(0),
        provider_specific_fields: None,
    }
}

pub fn chunk_parser(chunk: &Json) -> Result<GenericChunk, A2aError> {
    let Json::Object(fields) = chunk else {
        return Ok(EMPTY_CHUNK);
    };
    if let Some(Json::Object(error)) = fields.get("error") {
        let detail = error
            .get("message")
            .and_then(Json::as_str)
            .unwrap_or("Unknown error");
        return Err(get_error_class(
            &format!("A2A error: {detail}"),
            500,
            BTreeMap::new(),
        ));
    }
    Ok(build_chunk(chunk, fields))
}
