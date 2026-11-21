use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::responses_lifecycle_write::call::LifecycleOperation;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

// One delete or cancel call in one mode; the runs execute in order against the same case env
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub(crate) runs: Vec<Run>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    operation: LifecycleOperation,
    mode: Mode,
    response_id: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    extra_headers: Option<BTreeMap<String, String>>,
    // LiteLLM takes extra_query and never gives it to the handler
    #[serde(default)]
    extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    params: Params,
}

// The kwargs GenericLiteLLMParams and the pre-call logging update read
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub(crate) api_key: Option<String>,
    pub(crate) litellm_call_id: Option<String>,
    pub(crate) metadata: Option<Json>,
    pub(crate) litellm_metadata: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub(crate) runs: Vec<RunOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub(crate) result: Option<Json>,
    pub(crate) error: Option<Failure>,
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.runs.len() > limits.max_requests {
            return Err(SdkHttpError::Invalid(String::from(
                "the case scripts more runs than the request limit",
            )));
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
