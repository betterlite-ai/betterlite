use betterlite_http::proxy::limits::TransportLimits;
use serde_json::Value;

use crate::evals::error::EvalsError;

// no larger eval reply can reach a decoder, since the transport refuses a bigger body first
pub const MAX_EVAL_REPLY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

// LiteLLM's raw_response.json(): the bytes must parse before an operation's model validator runs
pub fn decode_eval_reply(bytes: &[u8]) -> Result<Value, EvalsError> {
    if bytes.len() > MAX_EVAL_REPLY_BYTES {
        return Err(EvalsError::ReplyTooLarge {
            size: bytes.len(),
            limit: MAX_EVAL_REPLY_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| EvalsError::ReplyJson { source })
}

mod tests;
