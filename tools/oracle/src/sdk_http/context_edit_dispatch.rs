use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::messages_context_clear::constants::COMPACT_SUMMARY_MAX_TOKENS;
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, UsageIteration,
};
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod runs;
mod support;

// one sdk_http case runs the dispatcher once per scenario against the loopback summary provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub scenarios: Vec<Scenario>,
}

// the caller's request as the case scripts it; the summary model settings and the counters are the harness's
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
    pub summary_model: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
    #[serde(default)]
    pub counter: Counter,
}

fn default_model() -> String {
    String::from("main-model")
}

const fn default_max_tokens() -> u64 {
    COMPACT_SUMMARY_MAX_TOKENS
}

// every token counter the request reaches answers the same number, as one scripted litellm.token_counter does
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counter {
    #[serde(default = "default_count")]
    pub count: usize,
}

const fn default_count() -> usize {
    50_001
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            count: default_count(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<Run>,
}

// one dispatch: what it returned or refused
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub result: Option<Shape>,
    pub failure: Option<Failure>,
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

// the exception the dispatcher let through: its class, its status when it carries one, and its text
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    pub status_code: Option<u16>,
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
