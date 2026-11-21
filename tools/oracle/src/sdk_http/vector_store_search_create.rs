use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::vector_search_execution::config::{Params, SearchQuery};
use betterlite_core::api::vector_store_search::service::Entry;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Create,
    Search,
}

// vector_store_create's five named optional arguments
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateArgs {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub file_ids: Option<Vec<String>>,
    #[serde(default)]
    pub expires_after: Option<Params>,
    #[serde(default)]
    pub chunking_strategy: Option<Params>,
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, String>>,
}

// vector_store_search's two required arguments and its four named optional ones
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    pub vector_store_id: String,
    pub query: SearchQuery,
    #[serde(default)]
    pub filters: Option<Params>,
    #[serde(default)]
    pub max_num_results: Option<i64>,
    #[serde(default)]
    pub ranking_options: Option<Params>,
    #[serde(default)]
    pub rewrite_query: Option<bool>,
}

// The caller's **kwargs: everything GenericLiteLLMParams receives from the oracle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOptions {
    #[serde(default)]
    pub api_key: Option<String>,
    pub litellm_call_id: String,
    #[serde(default)]
    pub mock_response: Option<Json>,
}

// One SDK call of either mode and the arguments that operation reads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub operation: Operation,
    pub mode: Entry,
    #[serde(default)]
    pub create: Option<CreateArgs>,
    #[serde(default)]
    pub search: Option<SearchArgs>,
    #[serde(default)]
    pub options: RunOptions,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<Params>,
    #[serde(default)]
    pub extra_query: Option<Params>,
    #[serde(default)]
    pub timeout_seconds: Option<f64>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
}

// The registry record and credential a search overlays, as LiteLLM's global registry holds them
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
    pub mode: Entry,
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

fn validate_run(run: &Run) -> Result<(), SdkHttpError> {
    if run.options.litellm_call_id.is_empty() {
        return Err(SdkHttpError::Invalid(
            "a run needs a litellm_call_id, so both sides log the same call".to_owned(),
        ));
    }
    let valid = match run.operation {
        Operation::Create => run.create.is_some(),
        Operation::Search => run.search.is_some(),
    };
    if valid {
        return Ok(());
    }
    Err(SdkHttpError::Invalid(
        "a run carries the arguments of its operation".to_owned(),
    ))
}
