use betterlite_anthropic::count_tokens::error::CountTokensError;
use betterlite_models::decisions::Json;

// LiteLLM hands response.json() back untouched, so any JSON value reaches the counter
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundryCountReply {
    pub document: Json,
}

pub fn decode_foundry_count_reply(bytes: &[u8]) -> Result<FoundryCountReply, CountTokensError> {
    serde_json::from_slice(bytes)
        .map(|document| FoundryCountReply { document })
        .map_err(|source| CountTokensError::Decode { source })
}
