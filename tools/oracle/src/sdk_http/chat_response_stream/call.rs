use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::sdk_http::ChatMessage;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::wrapper_stream::WireCall;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatResponseStreamCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) api_key: String,
    pub(crate) timeout_seconds: u64,
    pub(crate) request_input: String,
    #[serde(default)]
    pub(crate) responses_api_request: ResponsesRequest,
    pub(crate) runs: Vec<ChatRunSpec>,
}

// The Responses request keys LiteLLM's iterator reads; an absent key stays absent, so the created event sees null
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<IndexMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatRunSpec {
    pub(crate) mode: ChatRunMode,
    pub(crate) steps: Vec<ChatRunStep>,
}

// The iterator under test: its own __next__, or the async one that closes a reasoning segment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRunMode {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRunStep {
    Next,
    Drain,
}

impl ChatResponseStreamCall {
    // The params dict the Python iterator receives, with only the keys the case set
    pub(crate) fn params_json(&self) -> Result<Json, SdkHttpError> {
        let value =
            serde_json::to_value(&self.responses_api_request).map_err(SdkHttpError::Serialize)?;
        serde_json::from_value(value).map_err(SdkHttpError::Serialize)
    }
}

impl WireCall for ChatResponseStreamCall {
    fn model(&self) -> &str {
        &self.model
    }

    fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }
}
