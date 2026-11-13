use betterlite_models::decisions::Json;

use crate::limiter_records::{AUDIO_BYTES_PER_TOKEN, DEFAULT_AUDIO_TOKEN_ESTIMATE};

#[must_use]
pub fn estimate_audio_block_tokens(block: &Json) -> i64 {
    let Json::Object(fields) = block else {
        return DEFAULT_AUDIO_TOKEN_ESTIMATE;
    };
    let Some(Json::Object(audio)) = fields.get("input_audio") else {
        return DEFAULT_AUDIO_TOKEN_ESTIMATE;
    };
    let Some(Json::Text(payload)) = audio.get("data") else {
        return DEFAULT_AUDIO_TOKEN_ESTIMATE;
    };
    if payload.is_empty() {
        return DEFAULT_AUDIO_TOKEN_ESTIMATE;
    }
    // base64 length times 3 over 4 is the decoded byte count, and the payload is ASCII
    let decoded_bytes = i64::try_from(payload.len())
        .unwrap_or(i64::MAX)
        .saturating_mul(3)
        .div_euclid(4);
    decoded_bytes
        .div_euclid(AUDIO_BYTES_PER_TOKEN)
        .max(DEFAULT_AUDIO_TOKEN_ESTIMATE)
}

#[must_use]
pub fn estimate_audio_content_tokens(messages: Option<&Json>) -> i64 {
    let Some(Json::List(items)) = messages else {
        return 0;
    };
    let mut total: i64 = 0;
    for message in items {
        let Json::Object(fields) = message else {
            continue;
        };
        let Some(Json::List(content)) = fields.get("content") else {
            continue;
        };
        for block in content {
            if is_audio_block(block) {
                total = total.saturating_add(estimate_audio_block_tokens(block));
            }
        }
    }
    total
}

#[must_use]
pub fn strip_audio_content_blocks(messages: &Json) -> Json {
    let Json::List(items) = messages else {
        return messages.clone();
    };
    let mut stripped = Vec::with_capacity(items.len());
    for message in items {
        let Json::Object(fields) = message else {
            stripped.push(message.clone());
            continue;
        };
        let Some(Json::List(content)) = fields.get("content") else {
            stripped.push(message.clone());
            continue;
        };
        let mut kept: Vec<Json> = Vec::with_capacity(content.len());
        for block in content {
            if !is_audio_block(block) {
                kept.push(block.clone());
            }
        }
        let mut rebuilt = fields.clone();
        rebuilt.insert("content".to_owned(), Json::List(kept));
        stripped.push(Json::Object(rebuilt));
    }
    Json::List(stripped)
}

fn is_audio_block(block: &Json) -> bool {
    match block {
        Json::Object(fields) => {
            matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "input_audio")
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}
