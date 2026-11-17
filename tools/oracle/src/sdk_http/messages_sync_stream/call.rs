use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, UsageIteration,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::wrapper_stream::WireCall;
use crate::sdk_http::{ChatMessage, OracleLimits};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesSyncStreamCall {
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
    pub(crate) runs: Vec<SyncRunSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncRunSpec {
    #[serde(default)]
    pub(crate) initial: SyncInitial,
    pub(crate) steps: Vec<SyncStep>,
}

// The held buffers are dormant in this version, so a case seeds them to reach their branches
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncInitial {
    #[serde(default, rename = "holding_chunk")]
    pub(crate) held: Option<Json>,
    #[serde(default, rename = "holding_stop_reason_chunk")]
    pub(crate) held_stop_reason: Option<Json>,
    #[serde(default, rename = "queued_usage_chunk")]
    pub(crate) queued_usage: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStep {
    Next,
    Drain,
}

pub(crate) fn validate(
    call: &MessagesSyncStreamCall,
    limits: &OracleLimits,
) -> Result<(), SdkHttpError> {
    if call.timeout_seconds == 0 || call.timeout_seconds > limits.max_execution.as_secs() {
        return Err(SdkHttpError::Invalid(
            "timeout_seconds must be between 1 and the execution limit".to_owned(),
        ));
    }
    for run in &call.runs {
        for seed in [&run.initial.held, &run.initial.held_stop_reason]
            .into_iter()
            .flatten()
        {
            if !matches!(seed, Json::Object(_)) {
                return Err(SdkHttpError::Invalid(
                    "the initial held chunks must be objects".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

impl WireCall for MessagesSyncStreamCall {
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
