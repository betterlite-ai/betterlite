use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::wire::{
    ProviderField, StreamChoice, StreamDelta, StreamReply,
};
use serde_json::Map;
use uuid::Uuid;

use crate::invoke_events::error::InvokeStreamError;

pub(crate) fn chunk_parser(
    has_finished_thinking: &mut bool,
    chunk: &Json,
    next_id: fn() -> Uuid,
    now: fn() -> i64,
) -> Result<StreamReply, InvokeStreamError> {
    let Json::Object(fields) = chunk else {
        return Err(invalid_chunk());
    };
    let Some(Json::Text(generation)) = fields.get("generation") else {
        return Err(invalid_chunk());
    };
    let stop_reason = match fields.get("stop_reason") {
        Some(Json::Null) => None,
        Some(Json::Text(reason)) => Some(reason.clone()),
        Some(_) | None => return Err(invalid_chunk()),
    };
    let prompt_tokens = token_count(fields.get("prompt_token_count"))?;
    let generation_tokens = token_count(fields.get("generation_token_count"))?;
    let mut generated = generation.clone();
    if generated == "</think>" && !*has_finished_thinking {
        tracing::debug!("Deepseek r1: </think> received, setting has_finished_thinking to True");
        generated = String::new();
        *has_finished_thinking = true;
    }
    let content = if *has_finished_thinking {
        Some(generated.clone())
    } else {
        None
    };
    let reasoning_content = if *has_finished_thinking {
        None
    } else {
        Some(generated)
    };
    let total_tokens = prompt_tokens
        .checked_add(generation_tokens)
        .ok_or_else(invalid_chunk)?;
    let usage = serde_json::json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": generation_tokens,
        "total_tokens": total_tokens,
    });
    Ok(StreamReply {
        id: format!("chatcmpl-{}", (next_id)()),
        object: "chat.completion.chunk",
        created: (now)(),
        model: None,
        choices: vec![StreamChoice {
            index: 0,
            finish_reason: stop_reason,
            delta: StreamDelta {
                role: None,
                content,
                function_call: None,
                tool_calls: None,
                audio: None,
                reasoning_content,
                reasoning: ProviderField::Absent,
                passthrough: Map::new(),
            },
            logprobs: None,
        }],
        usage: Some(usage),
        service_tier: None,
    })
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
