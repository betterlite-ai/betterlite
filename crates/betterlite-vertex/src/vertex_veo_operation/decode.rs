use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

use super::error::VeoOperationError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// _VeoOperation: the fields the transforms read, with LiteLLM's default where a key is absent or null
#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VertexVeoOperation {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub name: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub done: bool,
    #[serde(default)]
    pub metadata: Option<Json>,
    #[serde(default)]
    pub response: Option<Json>,
    #[serde(default)]
    pub error: Option<Json>,
}

pub fn decode_vertex_veo_operation(bytes: &[u8]) -> Result<VertexVeoOperation, VeoOperationError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(VeoOperationError::ResponseTooLarge {
            size: bytes.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(VeoOperationError::Json)
}

// the usage and error dicts LiteLLM passes through unchanged, read back into the typed wire field
pub(crate) fn decoded_json<T>(value: Object) -> Result<T, VeoOperationError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(value).map_err(VeoOperationError::Encode)?)
        .map_err(VeoOperationError::Encode)
}
