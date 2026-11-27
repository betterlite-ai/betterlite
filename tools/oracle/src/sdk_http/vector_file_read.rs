use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::vector_search_execution::config::Params;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_checks::validate_timeout;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Retrieve,
    Content,
}

impl Operation {
    // the call_type LiteLLM logs for the read
    pub(super) const fn call_type(self) -> &'static str {
        match self {
            Self::Retrieve => "vector_store_file_retrieve",
            Self::Content => "vector_store_file_content",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

// The caller's **kwargs: the keys GenericLiteLLMParams and the logged params receive
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOptions {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub litellm_call_id: Option<String>,
    #[serde(default)]
    pub extra_query: Option<Params>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub max_retries: Option<i64>,
}

// One SDK call: retrieve/retrieve_content when sync, aretrieve/aretrieve_content when async
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub operation: Operation,
    pub mode: Mode,
    pub vector_store_id: String,
    pub file_id: String,
    #[serde(default)]
    pub options: RunOptions,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub timeout_seconds: Option<u32>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
}

// The registry record and credential a read overlays, as LiteLLM's global registry holds them
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub vector_store_id: String,
    pub credential_name: String,
    #[serde(default)]
    pub credential_values: Params,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
    #[serde(default)]
    pub registry: Option<Registry>,
}

// The logging object's state after update_from_kwargs, as both sides record it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub model: String,
    pub optional_params: Params,
    pub custom_llm_provider: String,
    pub litellm_call_id: Json,
    pub vector_store_id: Json,
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
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        for run in &self.runs {
            validate_run(run, limits)?;
        }
        if let Some(registry) = &self.registry
            && registry.credential_name.is_empty()
        {
            return Err(SdkHttpError::Invalid(
                "a registry credential needs a name".to_owned(),
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

fn validate_run(run: &Run, limits: &OracleLimits) -> Result<(), SdkHttpError> {
    if run.vector_store_id.is_empty() || run.file_id.is_empty() {
        return Err(SdkHttpError::Invalid(
            "a read needs a vector_store_id and a file_id".to_owned(),
        ));
    }
    if let Some(seconds) = run.timeout_seconds {
        validate_timeout(u64::from(seconds), limits)?;
    }
    Ok(())
}
