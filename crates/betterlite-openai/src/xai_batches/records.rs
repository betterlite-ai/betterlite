use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::batch_records::{BatchRecordsError, WireDecodeLimits};

use super::error::XaiBatchesError;

pub const XAI_BATCH_ID_PREFIX: &str = "batch_";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct XaiBatchState {
    #[serde(default)]
    pub num_requests: i64,
    #[serde(default)]
    pub num_pending: i64,
    #[serde(default)]
    pub num_success: i64,
    #[serde(default)]
    pub num_error: i64,
    #[serde(default)]
    pub num_cancelled: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct XaiBatch {
    pub batch_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub create_time: Option<String>,
    #[serde(default)]
    pub expire_time: Option<String>,
    #[serde(default)]
    pub cancel_time: Option<String>,
    #[serde(default)]
    pub cancel_by_xai_message: Option<String>,
    #[serde(default)]
    pub state: XaiBatchState,
    #[serde(default)]
    pub input_file_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct XaiBatchList {
    #[serde(default)]
    pub batches: Vec<XaiBatch>,
    #[serde(default)]
    pub pagination_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum XaiResultCode {
    Integer(i64),
    Text(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct XaiBatchResultError {
    #[serde(default)]
    pub code: Option<XaiResultCode>,
    #[serde(default)]
    pub message: String,
}

// Each request's answer is a one-key map named after its endpoint, with free-form JSON values
#[derive(Debug, Clone, Default, Deserialize)]
pub struct XaiBatchResultData {
    #[serde(default)]
    pub response: Option<IndexMap<String, IndexMap<String, Json>>>,
    #[serde(default)]
    pub error: Option<XaiBatchResultError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct XaiBatchResult {
    pub batch_request_id: String,
    #[serde(default)]
    pub batch_result: XaiBatchResultData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct XaiBatchResultsPage {
    #[serde(default)]
    pub results: Vec<XaiBatchResult>,
    #[serde(default)]
    pub pagination_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct XaiCreateBatchRequest {
    pub name: String,
    pub input_file_id: String,
}

#[must_use]
pub fn is_xai_batch_results_id(file_id: &str) -> bool {
    if file_id.starts_with(XAI_BATCH_ID_PREFIX) {
        return true;
    }
    false
}

pub fn decode_xai_batch<R>(bytes: &[u8], limits: &WireDecodeLimits) -> Result<R, XaiBatchesError>
where
    R: DeserializeOwned,
{
    if bytes.len() > limits.max_json_bytes {
        return Err(XaiBatchesError::Limit(BatchRecordsError::JsonTooLarge {
            limit: limits.max_json_bytes,
        }));
    }
    serde_json::from_slice(bytes).map_err(|source| XaiBatchesError::Decode { source })
}
