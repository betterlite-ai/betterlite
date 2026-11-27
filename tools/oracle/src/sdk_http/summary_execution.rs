use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::messages_context_clear::constants::COMPACT_SUMMARY_MAX_TOKENS;
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, UsageIteration,
};
use betterlite_core::api::summary_execution::edit_spec::CompactEditSpec;
use betterlite_core::api::summary_history::system::SystemPrompt;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod logs;
mod ports;
mod runs;
mod support;

// one sdk_http case runs the editor once per scenario against the loopback summary provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub model: String,
    pub messages: Vec<Json>,
    #[serde(default)]
    pub system: Option<SystemPrompt>,
    #[serde(default)]
    pub tools: Option<Vec<Json>>,
    pub edit_spec: CompactEditSpec,
    #[serde(default)]
    pub metadata: Option<Object>,
    #[serde(default)]
    pub summary_model: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
    #[serde(default)]
    pub principal: Option<Object>,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub counter: Counter,
    #[serde(default)]
    pub route: Route,
    #[serde(default)]
    pub record_steps: bool,
}

const fn default_max_tokens() -> u64 {
    COMPACT_SUMMARY_MAX_TOKENS
}

// the three async gates apply_compact_20260112 runs before the summary call, in this order
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default)]
    pub access: Verdict,
    #[serde(default)]
    pub budget: Verdict,
    #[serde(default)]
    pub rate: Verdict,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    #[default]
    Allow,
    Deny,
}

// what _count_effective_tokens answers: the conversation, the system text, or a counter exception
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counter {
    #[serde(default = "default_conversation_tokens")]
    pub conversation: usize,
    #[serde(default)]
    pub text: usize,
    #[serde(default)]
    pub fail: bool,
}

const fn default_conversation_tokens() -> usize {
    150_001
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            conversation: default_conversation_tokens(),
            text: 0,
            fail: false,
        }
    }
}

// the interface the editor reaches: an available router first, else the direct pipeline
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    #[default]
    Router,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<Run>,
}

// one editor call: what it returned or refused, the completion it made, its policy order and its log lines
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub result: Option<Shape>,
    pub failure: Option<Failure>,
    pub calls: Vec<CallRecord>,
    pub steps: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub messages: Vec<DecisionJson>,
    pub system: Option<DecisionJson>,
    pub applied_edits: Vec<AppliedEdit>,
    pub compaction_block: Option<CompactionBlock>,
    pub iterations_usage: Option<Vec<UsageIteration>>,
}

// AnthropicContextManagementError: the only failure that aborts the request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status_code: u16,
    pub message: String,
}

// the summary call as the completion port received it, before the provider resolves the model
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallRecord {
    pub route: Route,
    pub model: String,
    pub messages: Vec<Json>,
    pub max_tokens: u64,
    pub timeout_seconds: u64,
    pub metadata: Object,
    pub user: Option<String>,
    pub region: Option<String>,
}

// a compact_20260112 log line, with the exception text of the two variable warnings cut off
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub level: String,
    pub message: String,
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        _env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        runs::execute(self, api_base)
    }
}
