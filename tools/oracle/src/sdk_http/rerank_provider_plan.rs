use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::SyncDiagnostic;
use super::result::kinds::SdkResult;

mod native;

// One call of LiteLLM's rerank: the declared arguments, the remaining kwargs and the settings a proxy config carries
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub model: String,
    pub query: String,
    pub documents: Vec<RerankDocument>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub top_n: Option<i64>,
    #[serde(default)]
    pub rank_fields: Option<Vec<String>>,
    #[serde(default, with = "double_option")]
    pub return_documents: Option<Option<bool>>,
    #[serde(default)]
    pub max_chunks_per_doc: Option<i64>,
    #[serde(default)]
    pub max_tokens_per_doc: Option<i64>,
    #[serde(default)]
    pub kwargs: IndexMap<String, Json>,
    #[serde(default)]
    pub litellm_settings: Settings,
}

// litellm_settings as the dispatcher reads it; a `{mock}` value is the harness origin
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub togetherai_api_key: Option<String>,
    #[serde(default)]
    pub drop_params: Option<bool>,
}

// The SDK exception LiteLLM raises: a plain ValueError has no status, type, param or code at all
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

// The record or failure the call produced, plus the diagnostics it logged in order
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<Json>,
    pub failure: Option<Failure>,
    pub diagnostics: Vec<SyncDiagnostic>,
}

impl CallKind for Call {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
