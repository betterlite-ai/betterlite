use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use crate::openai_base_config::Headers;
use crate::ovhcloud_chat::error::OvhCloudError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddingReply {
    pub model: Option<Json>,
    pub data: Option<Json>,
    pub object: Option<Json>,
    pub usage: EmbeddingUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: i64,
    pub total_tokens: i64,
}

// the three passthrough fields and the counters are read as a typed record, where an absent or null part takes its default
#[derive(Debug, Default, Deserialize)]
struct WireReply {
    #[serde(default)]
    model: Option<Json>,
    #[serde(default)]
    data: Option<Json>,
    #[serde(default)]
    object: Option<Json>,
    #[serde(default)]
    usage: Option<WireUsage>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<i64>,
    #[serde(default)]
    total_tokens: Option<i64>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

// a body that is not a JSON object gives the exception with the body text, the upstream status and the headers
pub fn decode_embedding_reply(
    body: &[u8],
    status: u16,
    headers: &Headers,
) -> Result<EmbeddingReply, OvhCloudError> {
    let reply = serde_json::from_slice::<WireReply>(body).map_err(|_error| OvhCloudError {
        message: String::from_utf8_lossy(body).into_owned(),
        status_code: Some(status),
        headers: headers.clone(),
    })?;
    let usage = reply.usage.unwrap_or_default();
    Ok(EmbeddingReply {
        model: reply.model,
        data: reply.data,
        object: reply.object,
        usage: EmbeddingUsage {
            prompt_tokens: usage.prompt_tokens.unwrap_or_default(),
            total_tokens: usage.total_tokens.unwrap_or_default(),
        },
    })
}
