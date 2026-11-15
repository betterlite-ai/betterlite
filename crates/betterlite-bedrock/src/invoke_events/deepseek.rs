use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::wire::{
    ProviderField, StreamChoice, StreamDelta, StreamReply,
};
use serde_json::map;
use uuid::Uuid;

use crate::invoke_events::error::InvokeStreamError;

struct Fields {
    generation: String,
    finish_reason: Option<String>,
    prompt_tokens: i64,
    generation_tokens: i64,
}

pub(crate) fn chunk_parser(
    has_finished_thinking: &mut bool,
    chunk: &Json,
    next_id: fn() -> Uuid,
    now: fn() -> i64,
) -> Result<StreamReply, InvokeStreamError> {
    let fields = fields(chunk)?;
    let (content, reasoning_content) = generation_parts(has_finished_thinking, fields.generation);
    let total_tokens = fields
        .prompt_tokens
        .checked_add(fields.generation_tokens)
        .ok_or_else(invalid_chunk)?;
    Ok(StreamReply {
        id: format!("chatcmpl-{}", (next_id)()),
        object: "chat.completion.chunk",
        created: (now)(),
        model: None,
        choices: vec![StreamChoice {
            index: 0,
            finish_reason: fields.finish_reason,
            delta: StreamDelta {
                role: None,
                content,
                function_call: None,
                tool_calls: None,
                audio: None,
                reasoning_content,
                reasoning: ProviderField::Absent,
                passthrough: map::Map::new(),
            },
            logprobs: None,
        }],
        usage: Some(serde_json::json!({
            "prompt_tokens": fields.prompt_tokens,
            "completion_tokens": fields.generation_tokens,
            "total_tokens": total_tokens,
        })),
        service_tier: None,
    })
}

fn fields(chunk: &Json) -> Result<Fields, InvokeStreamError> {
    let Json::Object(values) = chunk else {
        return Err(invalid_chunk());
    };
    let Some(Json::Text(generation)) = values.get("generation") else {
        return Err(invalid_chunk());
    };
    let finish_reason = match values.get("stop_reason") {
        Some(Json::Null) => None,
        Some(Json::Text(reason)) if reason.is_empty() => None,
        Some(Json::Text(reason)) => Some(map_finish_reason(reason).reason.as_str().to_owned()),
        Some(_) | None => return Err(invalid_chunk()),
    };
    Ok(Fields {
        generation: generation.clone(),
        finish_reason,
        prompt_tokens: token_count(values.get("prompt_token_count"))?,
        generation_tokens: token_count(values.get("generation_token_count"))?,
    })
}

// the first </think> before the transition switches the stream from reasoning to content
fn generation_parts(
    has_finished_thinking: &mut bool,
    generation: String,
) -> (Option<String>, Option<String>) {
    if generation == "</think>" && !*has_finished_thinking {
        tracing::debug!("Deepseek r1: </think> received, setting has_finished_thinking to True");
        *has_finished_thinking = true;
        return (Some(String::new()), None);
    }
    if *has_finished_thinking {
        (Some(generation), None)
    } else {
        (None, Some(generation))
    }
}

// LiteLLM reads these with `or 0`, so absent and null count as zero and a negative count stays as sent.
fn token_count(value: Option<&Json>) -> Result<i64, InvokeStreamError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or_else(invalid_chunk),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(invalid_chunk())
        }
    }
}

fn invalid_chunk() -> InvokeStreamError {
    InvokeStreamError::Chunk {
        message: "the DeepSeek chunk is missing a required field".to_owned(),
    }
}
