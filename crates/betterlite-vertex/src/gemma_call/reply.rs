use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::compatible_wire::error::CompatibleWireError;
use betterlite_openai::compatible_wire::{ReplyOptions, decode_compatible_chat_reply};
use thiserror::Error;
use uuid::Uuid;

use crate::gemma_envelope::error::GemmaEnvelopeError;
use crate::gemma_envelope::response::unwrap_predictions;

#[derive(Debug, Error)]
pub enum GemmaReplyError {
    #[error("the Gemma reply is not valid JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Gemma predictions cannot be encoded as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Predictions(#[from] GemmaEnvelopeError),
    #[error(transparent)]
    Reply(#[from] CompatibleWireError),
}

impl GemmaReplyError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Decode { .. } | Self::Encode { .. } | Self::Reply(_) => 500,
            Self::Predictions(source) => source.status_code(),
        }
    }
}

pub fn decode_gemma_reply(
    bytes: &[u8],
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> Result<ChatResponse, GemmaReplyError> {
    let decoded: Json =
        serde_json::from_slice(bytes).map_err(|source| GemmaReplyError::Decode { source })?;
    let predictions = unwrap_predictions(&decoded)?;
    let encoded =
        serde_json::to_vec(&predictions).map_err(|source| GemmaReplyError::Encode { source })?;
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier,
        clock,
    };
    let reply = decode_compatible_chat_reply(&encoded, &options)?;
    Ok(reply.response)
}
