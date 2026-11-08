use crate::stream_chunk_shaping::ChunkShaper;
use crate::stream_chunk_shaping::StreamShapeError;
use crate::stream_chunk_shaping::creation::build_chunk;
use crate::stream_chunk_shaping::records::{
    ChunkArguments, JsonObject, JsonValue, StreamChunk, StreamOptions,
};

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
            shaper.holding_chunk.clone_from(&current);
            hold = true;
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
    let mut recent = shaper.chunks.iter().rev();
    let (Some(last), Some(previous)) = (recent.next(), recent.next()) else {
        return Ok(());
    };
    if last.choices.is_empty() || previous.choices.is_empty() {
        return Ok(());
    }
    let Some(repeated) = last
        .choices
        .first()
        .and_then(|choice| choice.delta.content.as_deref())
    else {
        shaper.repeated_messages_count = 1;
        return Ok(());
    };
    if repeated.chars().count() <= 2 {
        shaper.repeated_messages_count = 1;
        return Ok(());
    }
    let earlier = previous
        .choices
        .first()
        .and_then(|choice| choice.delta.content.as_deref());
    let same = earlier == Some(repeated);
    shaper.repeated_messages_count = if same {
        shaper.repeated_messages_count.saturating_add(1)
    } else {
        1
    };
    if shaper.repeated_messages_count < limit {
        return Ok(());
    }
    Err(StreamShapeError::Repetition {
        last_content: repeated.to_owned(),
        model: String::new(),
        provider: String::new(),
    })
}

pub fn finish_reason_handler(shaper: &mut ChunkShaper) -> StreamChunk {
    let mut model_response = build_chunk(shaper, ChunkArguments::default(), None);
    let reason = shaper
        .received_finish_reason
        .clone()
        .or_else(|| shaper.intermittent_finish_reason.clone())
        .unwrap_or_else(|| "stop".to_owned());
    let Some(choice) = model_response.choices.first_mut() else {
        return model_response;
    };
    choice.finish_reason = Some(match (reason.as_str(), shaper.tool_call) {
        ("stop", true) => "tool_calls".to_owned(),
        _ => reason,
    });
    model_response
}

// Whitespace-separated tables, compared one whole reason at a time.
const LENGTH_REASONS: &str = "max_tokens MAX_TOKENS compaction length";
const TOOL_CALL_REASONS: &str = "tool_use tool_calls";
const CONTENT_FILTER_REASONS: &str = "refusal ERROR_TOXIC SAFETY RECITATION LANGUAGE OTHER BLOCKLIST PROHIBITED_CONTENT SPII IMAGE_SAFETY IMAGE_PROHIBITED_CONTENT NO_IMAGE IMAGE_RECITATION IMAGE_OTHER ESCALATION sensitive guardrail_intervened content_filter content_filtered";
const STOP_REASONS: &str = "stop_sequence end_turn COMPLETE ERROR eos_token eos STOP FINISH_REASON_UNSPECIFIED MALFORMED_FUNCTION_CALL TOO_MANY_TOOL_CALLS MALFORMED_RESPONSE UNEXPECTED_TOOL_CALL MISSING_THOUGHT_SIGNATURE network_error stop error";

#[must_use]
pub fn mapped_finish_reason(raw: &str) -> &'static str {
    if LENGTH_REASONS.split(' ').any(|reason| reason == raw) {
        return "length";
    }
    if TOOL_CALL_REASONS.split(' ').any(|reason| reason == raw) {
        return "tool_calls";
    }
    if CONTENT_FILTER_REASONS
        .split(' ')
        .any(|reason| reason == raw)
    {
        return "content_filter";
    }
    if raw == "function_call" {
        return "function_call";
    }
    if !STOP_REASONS.split(' ').any(|reason| reason == raw) {
        tracing::warn!("Unmapped finish_reason '{raw}', defaulting to 'stop'");
    }
    "stop"
}

#[must_use]
pub const fn check_send_stream_usage(stream_options: Option<&StreamOptions>) -> bool {
    match stream_options {
        Some(options) => options.include_usage,
        None => false,
    }
}

#[must_use]
pub fn check_is_function_call(optional_params: Option<&JsonObject>) -> bool {
    optional_params
        .and_then(|params| params.get("functions"))
        .is_some_and(|functions| matches!(functions, JsonValue::Array(items) if !items.is_empty()))
}
