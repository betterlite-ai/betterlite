use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::messages_context_clear::constants::COMPACT_SUMMARY_MAX_TOKENS;
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, UsageIteration,
};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod runs;
mod support;

// one sdk_http case runs the prepare wrapper once per scenario against the loopback summary provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub scenarios: Vec<Scenario>,
}

// the caller's request as the case scripts it; the summary settings, the counters and the dispatcher are the harness's
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    #[serde(default = "default_model")]
    pub model: String,
    pub messages: Vec<Json>,
    #[serde(default)]
    pub system: Option<Json>,
    #[serde(default)]
    pub tools: Option<Vec<Json>>,
    #[serde(default)]
    pub context_management: Option<Json>,
    #[serde(default)]
    pub additional_drop_params: Option<Json>,
    #[serde(default)]
    pub metadata: Option<Object>,
    #[serde(default)]
    pub principal: Option<Object>,
    #[serde(default)]
    pub router: bool,
    #[serde(default)]
    pub dispatcher: Dispatcher,
    #[serde(default)]
    pub scripted: Option<Scripted>,
    #[serde(default)]
    pub summary_model: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
    #[serde(default = "default_count")]
    pub counter: usize,
}

fn default_model() -> String {
    String::from("main-model")
}

const fn default_max_tokens() -> u64 {
    COMPACT_SUMMARY_MAX_TOKENS
}

const fn default_count() -> usize {
    50_001
}

// what the wrapper's dispatcher answers: the real editors, a scripted result, nothing, or a broken editor
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dispatcher {
    #[default]
    Real,
    Result,
    Null,
    Error,
}

// the result a scripted dispatcher hands back, with no applied edits of its own
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scripted {
    pub messages: Vec<Json>,
    #[serde(default)]
    pub system: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<Run>,
}

// one wrapper call: what it returned or refused, and what the wrapper handed the dispatcher
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub result: Option<Shape>,
    pub failure: Option<Failure>,
    pub calls: Vec<CallRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub messages: Vec<Json>,
    pub system: Option<Json>,
    pub applied_edits: Vec<AppliedEdit>,
    pub compaction_block: Option<CompactionBlock>,
    pub iterations_usage: Option<Vec<UsageIteration>>,
}

// AnthropicContextManagementError: the only failure that aborts the request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    pub status_code: u16,
    pub message: String,
}

// the dispatcher's arguments as the wrapper passed them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallRecord {
    pub model: String,
    pub messages: Vec<Json>,
    pub tools: Option<Vec<Json>>,
    pub system: Option<Json>,
    pub spec: Option<Json>,
    pub metadata: Option<Object>,
    pub has_router: bool,
    pub has_principal: bool,
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
