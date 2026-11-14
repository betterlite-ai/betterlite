use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::Params;
use indexmap::IndexMap;
use serde_json::Number;

use super::config::text_param;
use super::error::S3VectorsSearchError;
use crate::s3_vectors_ingestion::S3VectorsInputError;

pub const DEFAULT_TOP_K: i64 = 5;

pub struct QueryInput<'request> {
    pub vector_store_id: &'request str,
    pub query_text: &'request str,
    pub query_vector: &'request [f64],
    pub max_num_results: Option<i64>,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryRequest {
    pub url: String,
    pub body: IndexMap<String, Json>,
    pub query_text: String,
}

pub fn build_query_request(input: &QueryInput<'_>) -> Result<QueryRequest, S3VectorsSearchError> {
    let (bucket_name, index_name) = split_store_id(input.vector_store_id, input.litellm_params)?;
    let mut floats = Vec::with_capacity(input.query_vector.len());
    for (index, value) in input.query_vector.iter().enumerate() {
        let number =
            Number::from_f64(*value).ok_or(S3VectorsSearchError::NonFiniteVector { index })?;
        floats.push(Json::Number(number));
    }
    let top_k = input.max_num_results.unwrap_or(DEFAULT_TOP_K);
    let mut body = IndexMap::new();
    body.insert(
        "vectorBucketName".to_owned(),
        Json::Text(bucket_name.to_owned()),
    );
    body.insert("indexName".to_owned(), Json::Text(index_name.to_owned()));
    body.insert(
        "queryVector".to_owned(),
        Json::Object(IndexMap::from([("float32".to_owned(), Json::List(floats))])),
    );
    body.insert("topK".to_owned(), Json::Number(Number::from(top_k)));
    body.insert("returnDistance".to_owned(), Json::Bool(true));
    body.insert("returnMetadata".to_owned(), Json::Bool(true));
    Ok(QueryRequest {
        url: format!("{}/QueryVectors", input.api_base),
        body,
        query_text: input.query_text.to_owned(),
    })
}

// B3458 keeps its split private, so the same rule lives here
fn split_store_id<'id>(
    store_id: &'id str,
    litellm_params: &'id Params,
) -> Result<(&'id str, &'id str), S3VectorsSearchError> {
    let (bucket_name, index_name) = match store_id.split_once(':') {
        Some(parts) => parts,
        None => (
            text_param(litellm_params, "vector_bucket_name")?
                .ok_or(S3VectorsInputError::StoreId)?,
            store_id,
        ),
    };
    if bucket_name.is_empty() || index_name.is_empty() {
        return Err(S3VectorsInputError::StoreId.into());
    }
    Ok((bucket_name, index_name))
}
