use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

mod operations;
mod responses;
mod tests;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    FilterOperatorMap,
    NestedAndOrFilters,
    KnowledgeBaseRetrieveUrl,
    UserContext,
    LocationFileIdName,
    AttributesAndScores,
    UnsupportedCreate,
    OptionMapping,
    RetrievalConfiguration,
    KnowledgeBaseResponseError,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    operators: Vec<String>,
    #[serde(default)]
    filters: Vec<Json>,
    #[serde(default)]
    urls: Vec<UrlScenario>,
    #[serde(default)]
    headers: Vec<(String, Option<String>)>,
    #[serde(default)]
    metadata: Vec<Option<IndexMap<String, Json>>>,
    #[serde(default)]
    responses: Vec<ResponseScenario>,
    #[serde(default)]
    options: Vec<OptionScenario>,
    #[serde(default)]
    contexts: Vec<ContextScenario>,
    #[serde(default)]
    requests: Vec<RequestScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionScenario {
    #[serde(default)]
    non_default_params: IndexMap<String, Json>,
    #[serde(default)]
    optional_params: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextScenario {
    #[serde(default)]
    extra_body: Option<Json>,
    #[serde(default)]
    litellm_params: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlScenario {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseScenario {
    status: u16,
    #[serde(default)]
    headers: Vec<(String, String)>,
    body: String,
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestScenario {
    vector_store_id: String,
    query: Json,
    #[serde(default)]
    optional_params: IndexMap<String, Json>,
    api_base: String,
    #[serde(default)]
    litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    extra_body: Option<Json>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(operations::run(input))
    }
}
