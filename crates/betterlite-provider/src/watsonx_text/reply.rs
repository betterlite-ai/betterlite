use std::str;

use betterlite_openai::chat::choice_finish_reason;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use super::error::WatsonxTextError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextGenerationResponse {
    pub text: String,
    pub finish_reason: String,
    pub created: i64,
    pub usage: GenerationUsage,
}

#[derive(Debug, Deserialize)]
pub struct TextGenerationReply {
    pub results: Vec<GenerationResult>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GenerationResult {
    pub generated_text: String,
    pub input_token_count: i64,
    pub generated_token_count: i64,
    pub stop_reason: String,
}

pub fn decode_text_generation_reply(bytes: &[u8]) -> Result<TextGenerationReply, WatsonxTextError> {
    serde_json::from_slice(bytes).map_err(|source| WatsonxTextError::InvalidResponse {
        body: body_text(bytes),
        source,
    })
}

pub fn transform_response(
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<TextGenerationResponse, WatsonxTextError> {
    let reply = decode_text_generation_reply(body)?;
    let Some(result) = reply.results.into_iter().next() else {
        return Err(WatsonxTextError::NoResults);
    };
    let created = match reply.created_at {
        Some(text) => read_timestamp(&text)?,
        None => now.timestamp(),
    };
    let (finish_reason, _) = choice_finish_reason(Some(&result.stop_reason));
    Ok(TextGenerationResponse {
        text: result.generated_text,
        finish_reason,
        created,
        usage: GenerationUsage {
            prompt_tokens: result.input_token_count,
            completion_tokens: result.generated_token_count,
            total_tokens: result
                .input_token_count
                .saturating_add(result.generated_token_count),
        },
    })
}

fn read_timestamp(text: &str) -> Result<i64, WatsonxTextError> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Ok(parsed.timestamp());
    }
    // LiteLLM's fromisoformat accepts a value without an offset; betterlite reads that as UTC
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f"))
        .map_err(WatsonxTextError::CreatedAt)?;
    Ok(naive.and_utc().timestamp())
}

fn body_text(bytes: &[u8]) -> String {
    let Ok(text) = str::from_utf8(bytes) else {
        return format!("{} bytes that are not UTF-8", bytes.len());
    };
    text.to_owned()
}
