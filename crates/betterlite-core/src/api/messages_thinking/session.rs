use serde::Deserialize;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary};

const OPENAI_MAX_PROMPT_CACHE_KEY_LENGTH: usize = 64;

// _ClaudeCodeUserId: only session_id is per conversation, and extra metadata keys are ignored.
#[derive(Debug, Deserialize)]
struct ClaudeCodeUserId {
    session_id: String,
}

#[derive(Debug, Error)]
pub enum SessionKeyError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
}

// prompt_cache_key_from_user_id: a plain user id names a person, so only its JSON session id becomes a key.
pub fn decode_messages_thinking(
    body: &[u8],
    limits: &ApiLimits,
) -> Result<Option<String>, SessionKeyError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: body.len(),
    }))?;
    let Ok(user_id) = serde_json::from_slice::<ClaudeCodeUserId>(body) else {
        return Ok(None);
    };
    let key: String = user_id
        .session_id
        .chars()
        .take(OPENAI_MAX_PROMPT_CACHE_KEY_LENGTH)
        .collect();
    Ok((!key.is_empty()).then_some(key))
}
