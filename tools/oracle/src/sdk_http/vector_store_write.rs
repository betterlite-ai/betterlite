use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreExpirationPolicy;
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

// One public SDK write: the named arguments update takes, the create fields it reads from kwargs, and the three overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub(crate) operation: Operation,
    pub(crate) mode: Mode,
    pub(crate) vector_store_id: String,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) expires_after: Option<VectorStoreExpirationPolicy>,
    #[serde(default)]
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) file_ids: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) chunking_strategy: Option<Json>,
    #[serde(default)]
    pub(crate) unknown: Option<Json>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_query: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
}

impl Run {
    // The create fields update reads out of **kwargs, in the order a caller typed them
    fn kwargs(&self) -> IndexMap<String, Json> {
        let mut kwargs = IndexMap::new();
        if let Some(file_ids) = &self.file_ids {
            kwargs.insert(
                String::from("file_ids"),
                Json::List(file_ids.iter().cloned().map(Json::Text).collect()),
            );
        }
        if let Some(chunking_strategy) = &self.chunking_strategy {
            kwargs.insert(String::from("chunking_strategy"), chunking_strategy.clone());
        }
        if let Some(unknown) = &self.unknown {
            kwargs.insert(String::from("unknown"), unknown.clone());
        }
        kwargs
    }
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
