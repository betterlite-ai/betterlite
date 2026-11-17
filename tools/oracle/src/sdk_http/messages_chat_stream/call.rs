use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, UsageIteration,
};
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::wrapper_stream::WireCall;
use crate::sdk_http::{ChatMessage, OracleLimits};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesChatStreamCall {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) api_key: String,
    pub(crate) timeout_seconds: u64,
    #[serde(default)]
    pub(crate) tool_name_mapping: IndexMap<String, String>,
    #[serde(default)]
    pub(crate) applied_edits: Vec<AppliedEdit>,
    #[serde(default)]
    pub(crate) compaction_block: Option<CompactionBlock>,
    #[serde(default)]
    pub(crate) iterations_usage: Option<Vec<UsageIteration>>,
    pub(crate) runs: Vec<ChatRunSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatRunSpec {
    #[serde(default)]
    pub(crate) initial: ChatInitial,
    pub(crate) mode: ChatMode,
    #[serde(default)]
    pub(crate) failure: Option<ChatFailureSpec>,
    pub(crate) steps: Vec<ChatStep>,
}

// The held buffers and the usage flag are dormant with a well-behaved provider, so a case seeds them
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatInitial {
    #[serde(default, rename = "holding_chunk")]
    pub(crate) held: Option<Json>,
    #[serde(default, rename = "holding_stop_reason_chunk")]
    pub(crate) held_stop_reason: Option<Json>,
    #[serde(default, rename = "queued_usage_chunk")]
    pub(crate) queued_usage: bool,
}

// The object a run drives: the wrapper's own __anext__, its async SSE encoder, or the byte view over both
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatMode {
    Wrapper,
    Sse,
    View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatStep {
    Next,
    Drain,
    Close,
}

// A case can fail the provider stream at a chosen pull, the way a provider error surfaces midstream
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatFailureSpec {
    pub(crate) kind: ChatFailureKind,
    #[serde(default = "default_status_code")]
    pub(crate) status_code: u16,
    #[serde(default)]
    pub(crate) message: String,
    pub(crate) after: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatFailureKind {
    // LiteLLM's BaseLLMException: its status and message become the error event
    Reported,
    // LiteLLM's bare Exception: the generic 500 with the incomplete stream wording
    Empty,
}

const fn default_status_code() -> u16 {
    500
}

pub(crate) fn validate(
    call: &MessagesChatStreamCall,
    limits: &OracleLimits,
) -> Result<(), SdkHttpError> {
    if call.timeout_seconds == 0 || call.timeout_seconds > limits.max_execution.as_secs() {
        return Err(SdkHttpError::Invalid(
            "timeout_seconds must be between 1 and the execution limit".to_owned(),
        ));
    }
    for run in &call.runs {
        if run.mode == ChatMode::Wrapper && run.steps.contains(&ChatStep::Close) {
            return Err(SdkHttpError::Invalid(
                "only a byte view can be closed".to_owned(),
            ));
        }
        for event in [&run.initial.held, &run.initial.held_stop_reason]
            .into_iter()
            .flatten()
        {
            if !matches!(event, Json::Object(_)) {
                return Err(SdkHttpError::Invalid(
                    "the initial held chunks must be objects".to_owned(),
                ));
            }
        }
        if let Some(failure) = &run.failure
            && failure.kind == ChatFailureKind::Reported
            && !(100..=599).contains(&failure.status_code)
        {
            return Err(SdkHttpError::Invalid(
                "a reported failure needs a status between 100 and 599".to_owned(),
            ));
        }
    }
    Ok(())
}

impl WireCall for MessagesChatStreamCall {
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

impl ChatFailureSpec {
    // The failure the provider stream raises, as B975 classifies it
    pub(crate) fn failure(&self) -> StreamFailure {
        match self.kind {
            ChatFailureKind::Reported => StreamFailure::Reported {
                status_code: self.status_code,
                message: self.message.clone(),
            },
            ChatFailureKind::Empty => StreamFailure::Unclassified {
                text: String::new(),
            },
        }
    }
}
