use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_vertex::vertex_relay_metadata::types::CompletionResponse;
use indexmap::IndexMap;

use super::super::gemini_passthrough_logging::assembly::{
    CollectedStreamError, complete_response, event_chunk, event_converter, gemini_chunks,
};
use super::super::gemini_passthrough_logging::counter::CollectedCounter;

// LiteLLM tests the generate route first and the rawPredict route second; any other route builds no response.
pub(super) fn vertex_response(
    lines: &[String],
    url_route: &str,
    model: &str,
    now: i64,
    counter: Option<&CollectedCounter>,
) -> Result<Option<Json>, CollectedStreamError> {
    let chunks =
        if url_route.contains("generateContent") || url_route.contains("streamGenerateContent") {
            gemini_chunks(lines, now)?
        } else if url_route.contains("rawPredict") || url_route.contains("streamRawPredict") {
            raw_predict_chunks(lines, now, counter)?
        } else {
            return Ok(None);
        };
    complete_response(chunks, model, now, counter)
}

// A fresh parser per call, so tool indexes restart at 0; an invalid or non-object line is skipped silently.
fn raw_predict_chunks(
    lines: &[String],
    now: i64,
    counter: Option<&CollectedCounter>,
) -> Result<Vec<Chunk>, CollectedStreamError> {
    let tokenizer = counter.ok_or(CollectedStreamError::Tokenizer)?;
    let mut converter = event_converter(tokenizer);
    let mut chunks = Vec::with_capacity(lines.len());
    for line in lines {
        let stripped = strip_sse_data_from_chunk(Some(line)).unwrap_or(line);
        let Ok(value) = serde_json::from_str::<Json>(stripped) else {
            continue;
        };
        if !matches!(value, Json::Object(_)) {
            continue;
        }
        chunks.push(event_chunk(&converter.chunk_parser(&value)?, now)?);
    }
    Ok(chunks)
}

// B3638's payload factory reads the response's id, model and usage, so the assembled body is projected to those.
pub(super) fn cost_record(body: &Json) -> CompletionResponse {
    CompletionResponse {
        id: None,
        model: body_model(body),
        usage: body_usage(body),
    }
}

pub(super) fn body_model(body: &Json) -> Option<String> {
    let Json::Object(fields) = body else {
        return None;
    };
    match fields.get("model") {
        Some(Json::Text(model)) if !model.is_empty() => Some(model.clone()),
        _other => None,
    }
}

// The price only needs the counts and the cached-read count, which the assembler writes as numbers.
fn body_usage(body: &Json) -> Option<CostUsage> {
    let Json::Object(fields) = body else {
        return None;
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        return None;
    };
    Some(CostUsage {
        prompt_tokens: count(usage, "prompt_tokens")?,
        completion_tokens: count(usage, "completion_tokens")?,
        total_tokens: count(usage, "total_tokens")?,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: count(usage, "cache_read_input_tokens"),
        numeric_extras: BTreeMap::new(),
    })
}

fn count(fields: &IndexMap<String, Json>, key: &str) -> Option<i64> {
    match fields.get(key)? {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
