use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::Attributes;
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
pub enum Operation {
    Create,
    List,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateArgs {
    pub file_id: String,
    #[serde(default)]
    pub attributes: Option<Attributes>,
    #[serde(default)]
    pub chunking_strategy: Option<IndexMap<String, Json>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub order: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOptions {
    #[serde(default)]
    pub api_key: Option<String>,
    pub litellm_call_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub operation: Operation,
    pub mode: Mode,
    pub vector_store_id: String,
    #[serde(default)]
    pub create: Option<CreateArgs>,
    #[serde(default)]
    pub list: Option<ListArgs>,
    #[serde(default)]
    pub options: RunOptions,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub timeout_seconds: Option<f64>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub unknown: Option<Json>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub vector_store_id: String,
    #[serde(default)]
    pub credential_values: IndexMap<String, Json>,
    #[serde(default)]
    pub failing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
    #[serde(default)]
    pub registry: Option<Registry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub operation: Operation,
    pub mode: Mode,
    pub result: Option<Json>,
    pub failure: Option<Failure>,
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
    let valid = match run.operation {
        Operation::Create => run.create.is_some(),
        Operation::List => run.list.is_some(),
    };
    if valid {
        return Ok(());
    }
    Err(SdkHttpError::Invalid(
        "a run carries the arguments of its operation".to_owned(),
    ))
}
