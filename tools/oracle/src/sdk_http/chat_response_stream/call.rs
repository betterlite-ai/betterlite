use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use crate::sdk_http::ChatMessage;
use crate::sdk_http::wrapper_stream::WireCall;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatResponseStreamCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) api_key: String,
    pub(crate) timeout_seconds: u64,
    pub(crate) request_input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) responses_api_request: Option<Json>,
    pub(crate) runs: Vec<ChatRunSpec>,
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
