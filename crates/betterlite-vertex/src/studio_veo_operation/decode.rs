#![expect(
    clippy::disallowed_types,
    reason = "LiteLLM keeps the provider's operation error dict unchanged as Dict[str, Any], an open provider field"
)]

use serde::Deserialize;
use serde_json::{Map, Value};
use serde_with::{DefaultOnNull, serde_as};

use super::error::StudioVeoOperationError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// GeminiLongRunningOperationResponse: name is required, done defaults to false and a present response is validated
#[serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct StudioVeoOperation {
    pub name: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub done: bool,
    #[serde(default)]
    pub metadata: Option<StudioVeoMetadata>,
    #[serde(default)]
    pub response: Option<StudioVeoResponse>,
    #[serde(default)]
    pub error: Option<Map<String, Value>>,
}

// GeminiOperationMetadata
#[derive(Debug, Clone, Deserialize)]
pub struct StudioVeoMetadata {
    #[serde(default, rename = "createTime")]
    pub create_time: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

// GeminiOperationResponse
#[derive(Debug, Clone, Deserialize)]
pub struct StudioVeoResponse {
    #[serde(rename = "generateVideoResponse")]
    pub generate_video_response: StudioGenerateVideoResponse,
}

// GeminiGenerateVideoResponse
#[derive(Debug, Clone, Deserialize)]
pub struct StudioGenerateVideoResponse {
    #[serde(rename = "generatedSamples")]
    pub generated_samples: Vec<StudioGeneratedSample>,
}

// GeminiGeneratedVideoSample
#[derive(Debug, Clone, Deserialize)]
pub struct StudioGeneratedSample {
    pub video: StudioVideoUri,
}

// GeminiVideoUri
#[derive(Debug, Clone, Deserialize)]
pub struct StudioVideoUri {
    pub uri: String,
}

pub fn decode_studio_veo_operation(
    bytes: &[u8],
) -> Result<StudioVeoOperation, StudioVeoOperationError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(StudioVeoOperationError::ResponseTooLarge {
            size: bytes.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(StudioVeoOperationError::Json)
}
