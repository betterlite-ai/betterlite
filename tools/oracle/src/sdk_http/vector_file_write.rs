use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::Attributes;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Update,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

// The vector store registry's answer for a run: the credentials it overlays, or the lookup that raises and is swallowed
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    #[serde(default)]
    pub(crate) credentials: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) failing: bool,
}

// One public SDK file write: the named arguments update takes, the overrides, and the kwargs a caller's own values become
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub(crate) operation: Operation,
    pub(crate) mode: Mode,
    pub(crate) vector_store_id: String,
    pub(crate) file_id: String,
    #[serde(default)]
    pub(crate) attributes: Option<Attributes>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) unknown: Option<Json>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) registry: Option<Registry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub(crate) runs: Vec<Run>,
}

// One write's answer: the SDK's return value, or the exception it raised, exactly as a caller sees it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub(crate) value: Option<Json>,
    pub(crate) failure: Option<Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub(crate) runs: Vec<RunOutcome>,
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
