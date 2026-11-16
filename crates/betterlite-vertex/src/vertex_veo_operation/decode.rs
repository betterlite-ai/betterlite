use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
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
    #[serde_as(as = "DefaultOnNull")]
    pub metadata: VeoOperationMetadata,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub response: VeoOperationResponse,
    #[expect(
        clippy::disallowed_types,
        reason = "LiteLLM copies the provider error dict unchanged into VideoObject.error (Dict[str, Any])"
    )]
    #[serde(default)]
    pub error: Option<Map<String, Value>>,
}

// _VeoOperationMetadata
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VeoOperationMetadata {
    #[serde(default, rename = "createTime")]
    pub create_time: Option<String>,
}

// _VeoOperationResponse
#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VeoOperationResponse {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub videos: Vec<VeoVideo>,
}

// _VeoVideo: gcsUri and mimeType are never read, so a gcsUri alone is not downloaded
#[serde_as]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VeoVideo {
    #[serde(default, rename = "bytesBase64Encoded")]
    #[serde_as(as = "DefaultOnNull")]
    pub bytes_base64_encoded: String,
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

// the usage dict LiteLLM builds as plain JSON, read back into the typed VideoObject field
pub(crate) fn decoded_json<T>(value: Object) -> Result<T, VeoOperationError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(value).map_err(VeoOperationError::Encode)?)
        .map_err(VeoOperationError::Encode)
}
