use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::Params;
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Retrieve,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

// The caller's **kwargs: what GenericLiteLLMParams reads and the call id both sides log
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOptions {
    #[serde(default)]
    pub api_key: Option<String>,
    pub litellm_call_id: String,
}

// One SDK read of either operation, its arguments and the kwargs retrieve and list share
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub operation: Operation,
    pub mode: Mode,
    #[serde(default)]
    pub vector_store_id: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    // absent: the signature's default, null: left out, value: sent
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub limit: Option<Option<i64>>,
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub order: Option<Option<String>>,
    #[serde(default)]
    pub options: RunOptions,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_query: Option<Params>,
    #[serde(default)]
    pub extra_body: Option<Params>,
    #[serde(default)]
    pub timeout: Option<f64>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
}

// The logging object's state after update_from_kwargs, as both sides record it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub model: String,
    pub optional_params: Params,
    pub custom_llm_provider: String,
    pub litellm_call_id: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub operation: Operation,
    pub mode: Mode,
    pub result: Option<Json>,
    pub failure: Option<super::result::Failure>,
    pub context: Option<Context>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<RunOutcome>,
}

impl CallKind for Call {
    fn validate(&self, _limits: &OracleLimits) -> Result<(), SdkHttpError> {
        for run in &self.runs {
            validate_run(run)?;
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

fn validate_run(run: &Run) -> Result<(), SdkHttpError> {
    if run.options.litellm_call_id.is_empty() {
        return Err(SdkHttpError::Invalid(
            "a run needs a litellm_call_id, so both sides log the same call".to_owned(),
        ));
    }
    if run.operation == Operation::Retrieve && run.vector_store_id.is_none() {
        return Err(SdkHttpError::Invalid(
            "a retrieve run carries a vector_store_id".to_owned(),
        ));
    }
    Ok(())
}
