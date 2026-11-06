use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    #[serde(rename = "TEXT")]
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointData {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRequest {
    pub application_id: String,
    pub checkpoint_id: String,
    pub actor: String,
    pub session_id: String,
    pub data_type: DataType,
    pub data: CheckpointData,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ModifiedData {
    pub content: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CheckpointResponse {
    pub action_type: Option<String>,
    pub modified_data: Option<ModifiedData>,
}

#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("the Ovalix Tracker request timed out")]
    Timeout {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Ovalix Tracker could not be reached")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Ovalix Tracker returned status {status}")]
    Status { status: StatusCode },
    #[error("the Ovalix Tracker response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Ovalix Tracker response is not valid JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Ovalix Tracker API key is not a valid header")]
    InvalidHeader {
        #[source]
        source: InvalidHeaderValue,
    },
}
