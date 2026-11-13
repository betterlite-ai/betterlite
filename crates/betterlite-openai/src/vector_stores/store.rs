use super::Request;
use super::error::VectorStoreError;
use crate::api::vector_store_records::records::vector::{
    VectorStoreChunkingStrategy, VectorStoreExpirationPolicy,
};
use crate::openai_base_config::metadata_filter_limit;
use betterlite_http::url_path::encode_segment;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Query {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    pub filters: Option<Value>,
    pub max_num_results: Option<i64>,
    pub ranking_options: Option<Value>,
    pub rewrite_query: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateParams {
    pub name: Option<String>,
    pub file_ids: Option<Vec<String>>,
    pub expires_after: Option<VectorStoreExpirationPolicy>,
    pub chunking_strategy: Option<VectorStoreChunkingStrategy>,
    pub metadata: Option<IndexMap<String, Value>>,
}

#[derive(Serialize)]
struct SearchBody<'request> {
    query: &'request Query,
    filters: &'request Option<Value>,
    max_num_results: Option<i64>,
    ranking_options: &'request Option<Value>,
    rewrite_query: Option<bool>,
}

#[derive(Serialize)]
struct CreateBody<'request> {
    name: &'request Option<String>,
    file_ids: &'request Option<Vec<String>>,
    expires_after: &'request Option<VectorStoreExpirationPolicy>,
    chunking_strategy: &'request Option<VectorStoreChunkingStrategy>,
    metadata: Option<IndexMap<String, String>>,
}

pub fn search_request(
    vector_store_id: &str,
    query: &Query,
    params: &SearchParams,
    api_base: &str,
) -> Result<Request, VectorStoreError> {
    let encoded = encode_segment(vector_store_id, "vector_store_id")?;
    let (base_url, separator, query_string) = match api_base.split_once('?') {
        Some((base_url, query_string)) => (base_url, "?", query_string),
        None => (api_base, "", ""),
    };
    Ok(Request {
        url: format!("{base_url}/{encoded}/search{separator}{query_string}"),
        body: json!(SearchBody {
            query,
            filters: &params.filters,
            max_num_results: params.max_num_results,
            ranking_options: &params.ranking_options,
            rewrite_query: params.rewrite_query,
        }),
    })
}

#[must_use]
pub fn create_request(params: &CreateParams, api_base: &str) -> Request {
    Request {
        url: api_base.to_owned(),
        body: json!(CreateBody {
            name: &params.name,
            file_ids: &params.file_ids,
            expires_after: &params.expires_after,
            chunking_strategy: &params.chunking_strategy,
            metadata: metadata_filter_limit(params.metadata.as_ref()),
        }),
    }
}
