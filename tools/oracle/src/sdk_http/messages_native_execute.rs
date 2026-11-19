use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod dispatch;
mod run;
mod scripted;

// anthropic_messages' named arguments as the case scripts them; the harness owns api_base
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryRequest {
    pub max_tokens: i64,
    pub messages: Vec<Json>,
    pub model: String,
    pub metadata: Option<Json>,
    pub stop_sequences: Option<Json>,
    pub stream: Option<bool>,
    pub system: Option<Json>,
    pub temperature: Option<Json>,
    pub thinking: Option<Json>,
    pub tool_choice: Option<Json>,
    pub tools: Option<Vec<Json>>,
    pub top_k: Option<Json>,
    pub top_p: Option<Json>,
    pub api_key: Option<String>,
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub kwargs: IndexMap<String, Json>,
}

// One CustomLogger the case installs, and what async_pre_request_hook does to the hook map
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookScript {
    Pass,
    Mutate { values: IndexMap<String, Json> },
    Replace { values: IndexMap<String, Json> },
}

// One WebSearchInterceptionLogger the case installs; a null answer lets the request continue
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchScript {
    #[serde(default)]
    pub answer: Option<Json>,
}

// One MessagesInterceptor the case installs; a null answer is the entry's None result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterceptorScript {
    pub can_handle: bool,
    pub answer: Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub request: EntryRequest,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub hooks: Vec<HookScript>,
    #[serde(default)]
    pub searches: Vec<SearchScript>,
    #[serde(default)]
    pub interceptors: Vec<InterceptorScript>,
}

// What the cache hook, the pre-hooks and the prepared dispatch observed; api_base is masked
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub cache: IndexMap<String, Json>,
    pub hooks: Vec<IndexMap<String, Json>>,
    pub dispatch: Option<IndexMap<String, Json>>,
    pub response: Option<Json>,
    pub stream: Vec<String>,
    pub failure: Option<Failure>,
}

// The exception @client re-raised, by the attributes LiteLLM's errors carry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    pub status: u16,
    #[serde(rename = "type")]
    pub error_type: Json,
    pub param: Json,
    pub code: Json,
}

impl Outcome {
    fn empty() -> Self {
        Self {
            cache: IndexMap::new(),
            hooks: Vec::new(),
            dispatch: None,
            response: None,
            stream: Vec::new(),
            failure: None,
        }
    }
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        run::send(self, api_base, env)
    }
}
