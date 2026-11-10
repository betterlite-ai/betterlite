use std::collections::BTreeMap;

use betterlite_openai::embedding::EmbeddingVector;
use betterlite_openai::mongodb_vector_store::Params;
use betterlite_openai::vector_stores::store::{Query, SearchParams};
use serde::Deserialize;
use serde_json::Value;

mod calls;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "call", content = "input")]
enum Call {
    #[serde(rename = "mongodb_validate_environment")]
    ValidateEnvironment(EnvironmentCall),
    #[serde(rename = "mongodb_complete_url")]
    CompleteUrl(UrlCall),
    #[serde(rename = "mongodb_prepare_search")]
    PrepareSearch(PrepareCall),
    #[serde(rename = "mongodb_search_request")]
    SearchRequest(SearchCall),
    #[serde(rename = "mongodb_decode_search")]
    DecodeSearch(DecodeCall),
    #[serde(rename = "mongodb_status_error")]
    StatusError(StatusCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    litellm_params: Option<Params>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    api_base: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareCall {
    litellm_params: Params,
    #[serde(default)]
    optional_params: SearchParams,
    extra_body: Option<Params>,
    query: Query,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchCall {
    litellm_params: Params,
    #[serde(default)]
    optional_params: SearchParams,
    extra_body: Option<Params>,
    query: Query,
    #[serde(default)]
    embedding: Vec<EmbeddingVector>,
    vector_store_id: String,
    api_base: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecodeCall {
    body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusCall {
    message: String,
    status_code: u16,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(Value::Array(
            input.calls.iter().map(calls::run_call).collect(),
        ))
    }
}

pub(crate) struct Op;
