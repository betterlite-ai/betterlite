use std::collections::BTreeMap;
use std::mem;

use betterlite_a2a::a2a_text::{A2aResponse, extract_text_from_a2a_response};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_translation::{StreamingScanKey, transform_user_key_metadata};

use super::{A2aTranslationError, paths};

const A2A_MAX_DEPTH: usize = 10;

struct ParsedChunk {
    original_index: usize,
    original_text: bool,
    object: GuardrailValue,
}

pub async fn process_streaming_response<G>(
    responses_so_far: &mut [GuardrailValue],
    guardrail: &G,
    logging_obj: Option<&mut GuardrailLogTarget>,
    user_api_key_dict: Option<&BTreeMap<String, GuardrailValue>>,
    request_data: Option<&mut BTreeMap<String, GuardrailValue>>,
) -> Result<(), A2aTranslationError>
where
    G: GuardrailContract,
{
    let mut chunks = parse_streaming_responses(responses_so_far);
    if chunks.is_empty() {
        return Ok(());
    }
    let (combined, text_indices) = collect_texts_from_chunks(&chunks)?;
    if combined.is_empty() {
        return Ok(());
    }
    let mut fresh_request_data = BTreeMap::new();
    let prepared_request_data = request_data.unwrap_or(&mut fresh_request_data);
    add_stream_context(prepared_request_data, responses_so_far, user_api_key_dict);
    let inputs = GuardrailApiInputs {
        texts: Some(vec![combined]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    let guardrailed = guardrail
        .apply_guardrail(
            inputs,
            prepared_request_data,
            GuardrailInputType::Response,
            logging_obj,
        )
        .await?;
    let Some(returned) = guardrailed.texts else {
        return Ok(());
    };
    let Some(first_text) = returned.first() else {
        return Ok(());
    };
    let first_chunk = text_indices.first().copied();
    write_chunks(responses_so_far, &mut chunks, first_chunk, first_text)
}

fn write_chunks(
    responses_so_far: &mut [GuardrailValue],
    chunks: &mut [ParsedChunk],
    first_chunk: Option<usize>,
    first_text: &str,
) -> Result<(), A2aTranslationError> {
    for chunk in chunks {
        apply_stream_rewrites(chunk, first_chunk, first_text);
        let Some(slot) = responses_so_far.get_mut(chunk.original_index) else {
            continue;
        };
        let object = mem::replace(&mut chunk.object, GuardrailValue::Null);
        *slot = if chunk.original_text {
            GuardrailValue::Text(compact_line(&object)?)
        } else {
            object
        };
    }
    Ok(())
}

#[must_use]
pub fn get_streaming_scan_key(responses_so_far: &[GuardrailValue]) -> StreamingScanKey {
    let mut combined = String::new();
    for chunk in parse_streaming_responses(responses_so_far) {
        if let Ok(text) = a2a_response_text(&chunk.object) {
            combined.push_str(&text);
        }
    }
    StreamingScanKey {
        texts: vec![combined],
        tool_calls: Vec::new(),
        stream_ended: false,
        tool_calls_in_flight: false,
    }
}

fn add_stream_context(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    responses_so_far: &[GuardrailValue],
    user_api_key_dict: Option<&BTreeMap<String, GuardrailValue>>,
) {
    request_data
        .entry("responses_so_far".to_owned())
        .or_insert_with(|| GuardrailValue::Array(responses_so_far.to_vec()));
    if request_data.contains_key("litellm_metadata") {
        return;
    }
    let metadata = transform_user_key_metadata(user_api_key_dict);
    if metadata.is_empty() {
        return;
    }
    request_data.insert(
        "litellm_metadata".to_owned(),
        GuardrailValue::Object(metadata),
    );
}

fn parse_streaming_responses(responses_so_far: &[GuardrailValue]) -> Vec<ParsedChunk> {
    let mut parsed = Vec::new();
    for (index, item) in responses_so_far.iter().enumerate() {
        let (object, original_text) = match item {
            GuardrailValue::Object(_) => (item.clone(), false),
            GuardrailValue::Text(text) => {
                let Ok(value) = serde_json::from_str::<GuardrailValue>(text.trim()) else {
                    continue;
                };
                (value, true)
            }
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_) => continue,
        };
        let GuardrailValue::Object(fields) = &object else {
            continue;
        };
        if !matches!(fields.get("result"), Some(GuardrailValue::Object(_))) {
            continue;
        }
        parsed.push(ParsedChunk {
            original_index: index,
            original_text,
            object,
        });
    }
    parsed
}

fn collect_texts_from_chunks(
    chunks: &[ParsedChunk],
) -> Result<(String, Vec<usize>), A2aTranslationError> {
    let mut combined = String::new();
    let mut text_indices = Vec::new();
    for chunk in chunks {
        let text = a2a_response_text(&chunk.object)?;
        if text.is_empty() {
            continue;
        }
        combined.push_str(&text);
        text_indices.push(chunk.original_index);
    }
    Ok((combined, text_indices))
}

fn a2a_response_text(object: &GuardrailValue) -> Result<String, A2aTranslationError> {
    let shape =
        serde_json::to_value(object).map_err(|source| A2aTranslationError::Json { source })?;
    let response: A2aResponse =
        serde_json::from_value(shape).map_err(|source| A2aTranslationError::Json { source })?;
    extract_text_from_a2a_response(&response, A2A_MAX_DEPTH).map_err(A2aTranslationError::from)
}

fn apply_stream_rewrites(chunk: &mut ParsedChunk, first_chunk: Option<usize>, first_text: &str) {
    let is_first = first_chunk == Some(chunk.original_index);
    let GuardrailValue::Object(fields) = &mut chunk.object else {
        return;
    };
    let Some(GuardrailValue::Object(result)) = fields.get("result") else {
        return;
    };
    let placements = paths::extract_texts_from_result(result);
    if placements.is_empty() {
        return;
    }
    let Some(GuardrailValue::Object(mutable_result)) = fields.get_mut("result") else {
        return;
    };
    for (position, placement) in placements.iter().enumerate() {
        let text = if is_first && position == 0 {
            first_text
        } else {
            ""
        };
        paths::apply_text_to_path(mutable_result, &placement.path, placement.part_index, text);
    }
}

fn compact_line(object: &GuardrailValue) -> Result<String, A2aTranslationError> {
    let text =
        serde_json::to_string(object).map_err(|source| A2aTranslationError::Json { source })?;
    Ok(format!("{text}\n"))
}

mod tests;
