use std::str::Utf8Error;
use std::str::from_utf8;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary};

mod lines;
pub mod records;
mod tests;

const RESPONSE_COMPLETED: &str = "response.completed";
const OUTPUT_ITEM_DONE: &str = "response.output_item.done";
const OUTPUT_TEXT_DONE: &str = "response.output_text.done";

#[derive(Debug, Error)]
pub enum ResponsesSseRecoveryError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the recorded SSE line is not UTF-8")]
    Utf8(#[from] Utf8Error),
}

// parse_sse_json_chunk: one recorded line yields its JSON object, and anything else yields nothing
pub fn decode_recovery_line(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<Option<IndexMap<String, Json>>, ResponsesSseRecoveryError> {
    limits.check(ApiBoundariesInput::Event(EventBoundary {
        known_bytes: bytes.len(),
    }))?;
    let text = from_utf8(bytes)?;
    let Some(stripped) = strip_sse_data_from_chunk(Some(lines::trim_line(text))) else {
        return Ok(None);
    };
    // [DONE], blanks and event: lines are no JSON document, so the strict parse refuses them
    match serde_json::from_str::<Json>(lines::trim_line(stripped)) {
        Ok(Json::Object(fields)) => Ok(Some(fields)),
        Ok(_) | Err(_) => Ok(None),
    }
}

// _recover_output_items_from_raw_sse: one recorded line at a time, with a completed event ending recovery
pub fn recover_recorded_output(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<Vec<Json>, ResponsesSseRecoveryError> {
    let text = from_utf8(bytes)?;
    let mut output_items = records::OutputItems::new();
    let mut text_only_items = records::TextOnlyItems::new();
    for line in text.split(['\r', '\n']) {
        let Some(parsed) = decode_recovery_line(line.as_bytes(), limits)? else {
            continue;
        };
        match event_kind(&parsed) {
            Some(RESPONSE_COMPLETED) => {
                if let Some(output) = records::completed_event_output(&parsed, limits)? {
                    return Ok(output);
                }
            }
            Some(OUTPUT_ITEM_DONE) => {
                records::record_output_item_chunk(&parsed, &mut output_items, limits)?;
            }
            Some(OUTPUT_TEXT_DONE) => {
                records::record_output_text_chunk(
                    &parsed,
                    &output_items,
                    &mut text_only_items,
                    limits,
                )?;
            }
            Some(_) | None => {}
        }
    }
    records::merged_output_items(&output_items, &text_only_items, limits)
}

// _recover_output_items_from_logging: only a recorded string has anything to recover
pub fn recover_logged_output(
    logging: &LoggingState,
    limits: &ApiLimits,
) -> Result<Vec<Json>, ResponsesSseRecoveryError> {
    let Some(Json::Text(original)) = logging.model_call_details.get("original_response") else {
        return Ok(Vec::new());
    };
    recover_recorded_output(original.as_bytes(), limits)
}

// a non-string type is no event LiteLLM recognizes, and neither is a missing type
fn event_kind(parsed: &IndexMap<String, Json>) -> Option<&str> {
    if let Some(Json::Text(kind)) = parsed.get("type") {
        return Some(kind.as_str());
    }
    None
}
