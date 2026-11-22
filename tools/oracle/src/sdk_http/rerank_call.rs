use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;

mod native;
mod report;
mod support;

// A `{mock}` value names the harness origin in env and kwargs alike
const MOCK: &str = "{mock}";

// One case of LiteLLM's arerank: several wrapper calls against the same mock origin
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
}

// The eight declared arguments and the remaining kwargs of one arerank call
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub model: String,
    pub query: String,
    pub documents: Vec<RerankDocument>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub top_n: Option<i64>,
    #[serde(default)]
    pub rank_fields: Option<Vec<String>>,
    // absent and explicit null both reach arerank as its None default, which omits return_documents from the request
    #[serde(default)]
    pub return_documents: Option<bool>,
    #[serde(default)]
    pub max_chunks_per_doc: Option<i64>,
    #[serde(default)]
    pub kwargs: IndexMap<String, Json>,
}

// The SDK exception arerank raises: a plain ValueError has no status, type, param or code at all
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub param: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    pub message: String,
}

// What each wrapper call produced, in the order the case made them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<RunOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub result: Option<Json>,
    pub failure: Option<Failure>,
}

impl CallKind for Call {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.runs.is_empty() {
            return Err(SdkHttpError::Invalid(
                "a rerank call needs at least one run".to_owned(),
            ));
        }
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
