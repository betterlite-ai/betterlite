use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;

mod native;
mod report;
mod support;

// A `{mock}` value names the harness origin in kwargs and api_base alike
const MOCK: &str = "{mock}";

// One case of LiteLLM's decisions and adecisions: several wrapper calls against the same mock origin
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
}

// The declared arguments and the remaining kwargs of one decisions call; state travels in kwargs the way litellm.decisions takes it
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub model: String,
    pub input: Option<Json>,
    pub questions: Option<Json>,
    pub safety_identifier: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub timeout: Option<f64>,
    pub extra_headers: Option<BTreeMap<String, String>>,
    // adecisions awaits, decisions blocks; the case picks which wrapper it calls
    #[serde(default)]
    pub asynchronous: bool,
    #[serde(default)]
    pub kwargs: IndexMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    pub status: Option<u16>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
    pub message: String,
}

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
                "a decisions call needs at least one run".to_owned(),
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
