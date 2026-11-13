use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::api::batch_records::WireDecodeLimits;

use super::error::XaiFilesError;

// LiteLLM's XAIFile: purpose is stored by xAI but never read, every projection reports batch
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct XaiFile {
    pub id: String,
    #[serde(default)]
    pub bytes: i64,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct XaiFileList {
    #[serde(default)]
    pub data: Vec<XaiFile>,
    #[serde(default)]
    pub pagination_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct XaiFileDeleted {
    pub id: String,
    #[serde(default = "default_deleted")]
    pub deleted: bool,
}

const fn default_deleted() -> bool {
    true
}

pub fn decode_xai_upload_reply<R>(
    bytes: &[u8],
    limits: &WireDecodeLimits,
) -> Result<R, XaiFilesError>
where
    R: DeserializeOwned,
{
    if bytes.len() > limits.max_json_bytes {
        return Err(XaiFilesError::TooLarge {
            limit: limits.max_json_bytes,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| XaiFilesError::Decode { source })
}
