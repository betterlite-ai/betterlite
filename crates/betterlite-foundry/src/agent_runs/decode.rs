use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use thiserror::Error;

use crate::agent_results::messages::MessagesData;

pub const RUN_COMPLETED: &str = "completed";
pub const RUN_FAILED: &str = "failed";
pub const RUN_CANCELLED: &str = "cancelled";
pub const RUN_EXPIRED: &str = "expired";
pub const UNKNOWN_ERROR: &str = "Unknown error";

// the run status, the created thread and run, and the message list arrive as provider JSON over the wire
#[derive(Debug, Error)]
pub enum AgentRunDecodeError {
    #[error("the agent response is not valid JSON")]
    Invalid {
        #[source]
        source: serde_json::Error,
    },
    #[error("the agent response is not a JSON object")]
    NotObject,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentRunStatus {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub last_error: Option<RunError>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunError {
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatedObject {
    pub id: String,
}

pub fn decode_agent_run_status(bytes: &[u8]) -> Result<AgentRunStatus, AgentRunDecodeError> {
    match serde_json::from_slice::<AgentRunStatus>(bytes) {
        Ok(status) => Ok(status),
        Err(source) => Err(classify(bytes, source)),
    }
}

pub fn decode_created_id(bytes: &[u8]) -> Result<String, AgentRunDecodeError> {
    match serde_json::from_slice::<CreatedObject>(bytes) {
        Ok(created) => Ok(created.id),
        Err(source) => Err(classify(bytes, source)),
    }
}

pub fn decode_agent_messages(bytes: &[u8]) -> Result<MessagesData, AgentRunDecodeError> {
    match serde_json::from_slice::<MessagesData>(bytes) {
        Ok(messages) => Ok(messages),
        Err(source) => Err(classify(bytes, source)),
    }
}

// a typed decode failure on a JSON scalar is not the same as malformed bytes, and the two read differently
fn classify(bytes: &[u8], source: serde_json::Error) -> AgentRunDecodeError {
    match serde_json::from_slice::<Json>(bytes) {
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            AgentRunDecodeError::NotObject
        }
        Ok(Json::Object(_)) | Err(_) => AgentRunDecodeError::Invalid { source },
    }
}
