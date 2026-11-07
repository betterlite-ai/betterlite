use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::creation::model_response_creator;
use crate::stream_chunk_shaping::error::StreamShapeError;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, StreamChunk, StreamOptions};

// Template tokens that sagemaker and hf streams may split across chunks.
const SPECIAL_TOKENS: [&str; 7] = [
    "<|assistant|>",
    "<|system|>",
    "<|user|>",
    "<s>",
    "</s>",
    "<|im_end|>",
    "<|im_start|>",
];

#[must_use]
pub fn check_special_tokens(
    shaper: &mut ChunkShaper,
    chunk: &str,
    finish_reason: Option<&str>,
) -> (bool, String) {
    if shaper.provider.as_deref() != Some("sagemaker") {
        return (false, chunk.to_owned());
    }
    if finish_reason.is_some_and(|reason| !reason.is_empty()) {
        let mut cleaned = chunk.to_owned();
        for token in SPECIAL_TOKENS {
            cleaned = cleaned.replace(token, "");
        }
        return (false, cleaned);
    }
    if shaper.progress.sent_first_chunk {
        return (false, chunk.to_owned());
    }
    let current = format!("{}{}", shaper.holding_chunk, chunk)
        .trim()
        .to_owned();
    let mut hold = false;
    for token in SPECIAL_TOKENS {
        let short = current.chars().count() < token.chars().count();
        if short && token.contains(&current) {
            hold = true;
            shaper.holding_chunk.clone_from(&current);
            continue;
        }
        if !short && current.contains(token) {
            shaper.holding_chunk = current.replace(token, "");
            hold = true;
        }
    }
    if !hold {
        shaper.holding_chunk.clear();
    }
    (hold, current)
}

pub fn raise_on_model_repetition(
    shaper: &mut ChunkShaper,
    limit: usize,
) -> Result<(), StreamShapeError> {
    if shaper.chunks.len() < 2 {
        return Ok(());
    }
    let mut recent = shaper.chunks.iter().rev();
    let (Some(last), Some(previous)) = (recent.next(), recent.next()) else {
        return Ok(());
    };
    if last.choices.is_empty() || previous.choices.is_empty() {
        return Ok(());
    }
    let last_content = last
        .choices
        .first()
        .and_then(|choice| choice.delta.content.as_deref());
    let Some(latest) = last_content else {
        shaper.repeated_messages_count = 1;
        return Ok(());
    };
    if latest.chars().count() <= 2 {
        shaper.repeated_messages_count = 1;
        return Ok(());
    }
    let previous_content = previous
        .choices
        .first()
        .and_then(|choice| choice.delta.content.as_deref());
    if previous_content == Some(latest) {
        shaper.repeated_messages_count = shaper.repeated_messages_count.saturating_add(1);
    } else {
        shaper.repeated_messages_count = 1;
    }
    if shaper.repeated_messages_count < limit {
        return Ok(());
    }
    Err(StreamShapeError::Repetition {
        last_content: latest.to_owned(),
        model: String::new(),
        provider: String::new(),
    })
}

pub fn finish_reason_handler(shaper: &mut ChunkShaper) -> Result<StreamChunk, StreamShapeError> {
    let mut model_response = model_response_creator(shaper, None, None)?;
    let finish_reason = shaper
        .received_finish_reason
        .clone()
        .or_else(|| shaper.intermittent_finish_reason.clone())
        .unwrap_or_else(|| "stop".to_owned());
    let Some(choice) = model_response.choices.first_mut() else {
        return Ok(model_response);
    };
    choice.finish_reason = Some(match (finish_reason.as_str(), shaper.tool_call) {
        ("stop", true) => "tool_calls".to_owned(),
        _ => finish_reason,
    });
    Ok(model_response)
}

const LENGTH_REASONS: [&str; 4] = ["max_tokens", "MAX_TOKENS", "compaction", "length"];
const TOOL_CALL_REASONS: [&str; 2] = ["tool_use", "tool_calls"];
const CONTENT_FILTER_REASONS: [&str; 19] = [
    "refusal",
    "ERROR_TOXIC",
    "SAFETY",
    "RECITATION",
    "LANGUAGE",
    "OTHER",
    "BLOCKLIST",
    "PROHIBITED_CONTENT",
    "SPII",
    "IMAGE_SAFETY",
    "IMAGE_PROHIBITED_CONTENT",
    "NO_IMAGE",
    "IMAGE_RECITATION",
    "IMAGE_OTHER",
    "ESCALATION",
    "sensitive",
    "guardrail_intervened",
    "content_filter",
    "content_filtered",
];
const STOP_REASONS: [&str; 16] = [
    "stop_sequence",
    "end_turn",
    "COMPLETE",
    "ERROR",
    "eos_token",
    "eos",
    "STOP",
    "FINISH_REASON_UNSPECIFIED",
    "MALFORMED_FUNCTION_CALL",
    "TOO_MANY_TOOL_CALLS",
    "MALFORMED_RESPONSE",
    "UNEXPECTED_TOOL_CALL",
    "MISSING_THOUGHT_SIGNATURE",
    "network_error",
    "stop",
    "error",
];

#[must_use]
pub fn mapped_finish_reason(raw: &str) -> &'static str {
    if LENGTH_REASONS.contains(&raw) {
        return "length";
    }
    if TOOL_CALL_REASONS.contains(&raw) {
        return "tool_calls";
    }
    if CONTENT_FILTER_REASONS.contains(&raw) {
        return "content_filter";
    }
    if raw == "function_call" {
        return "function_call";
    }
    if !STOP_REASONS.contains(&raw) {
        tracing::warn!("Unmapped finish_reason '{raw}', defaulting to 'stop'");
    }
    "stop"
}

#[must_use]
pub const fn check_send_stream_usage(stream_options: Option<&StreamOptions>) -> bool {
    let Some(options) = stream_options else {
        return false;
    };
    options.include_usage
}

#[must_use]
pub fn check_is_function_call(optional_params: Option<&JsonObject>) -> bool {
    let Some(params) = optional_params else {
        return false;
    };
    let Some(functions) = params.get("functions") else {
        return false;
    };
    matches!(functions, JsonValue::Array(items) if !items.is_empty())
}
