use std::collections::BTreeMap;

use betterlite_openai::milvus_vector_store::OptionalParams;
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
#[serde(tag = "call", content = "input", rename_all = "snake_case")]
enum Call {
    MilvusAuthCredentials(MilvusParamsCall),
    MilvusValidateEnvironment(MilvusEnvironmentCall),
    MilvusCompleteUrl(MilvusUrlCall),
    MilvusMapParams(MilvusMapCall),
    MilvusSearchRequest(MilvusSearchCall),
    MilvusDecodeSearch(MilvusDecodeCall),
    MilvusCreateRequest(MilvusCreateCall),
    MilvusCreateResponse(MilvusCreateResponseCall),
    PgVectorValidateEnvironment(PgKeyCall),
    PgVectorCompleteUrl(PgUrlCall),
    PgVectorSearchRequest(PgSearchCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilvusEnvironmentCall {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    litellm_params: Option<MilvusParamsCall>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MilvusParamsCall {
    api_key: Option<String>,
    milvus_db_name: Option<String>,
    milvus_partition_names: Option<Vec<String>>,
    milvus_text_field: Option<String>,
    litellm_embedding_model: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilvusUrlCall {
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilvusMapCall {
    #[serde(default)]
    non_default_params: OptionalParams,
    #[serde(default)]
    optional_params: OptionalParams,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilvusSearchCall {
    vector_store_id: String,
    query_text: String,
    query_vector: Vec<f64>,
    #[serde(default)]
    optional_params: OptionalParams,
    api_base: String,
    #[serde(default)]
    litellm_params: MilvusParamsCall,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilvusDecodeCall {
    status_code: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    body: String,
    query_text: String,
    text_field: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilvusCreateCall {
    #[serde(default)]
    optional_params: OptionalParams,
    api_base: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilvusCreateResponseCall {
    status_code: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PgKeyCall {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PgUrlCall {
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PgSearchCall {
    vector_store_id: String,
    query: Query,
    #[serde(default)]
    params: SearchParams,
    api_base: String,
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
