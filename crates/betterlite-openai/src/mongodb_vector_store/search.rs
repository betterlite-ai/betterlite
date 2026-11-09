use betterlite_models::decisions::Json;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Serialize;
use serde_json::Number;

use super::Params;
use super::error::MongoDbError;
use super::prepare::PreparedSearch;
use crate::embedding::{EmbeddingResponse, EmbeddingVector};

const NO_EMBEDDING: &str = "The embedding model returned no embedding for the search query. Check litellm_embedding_model.";
const INVALID_VECTOR: &str = "The embedding model must return a non-empty, finite query vector.";
// Python's quote(value, safe="") keeps only the RFC 3986 unreserved characters
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchRequest {
    pub url: String,
    pub body: Params,
}

pub fn search_request(
    prepared: &PreparedSearch,
    embedding: &EmbeddingResponse,
    vector_store_id: &str,
    api_base: &str,
) -> Result<SearchRequest, MongoDbError> {
    let Some(record) = embedding.data.first() else {
        return Err(MongoDbError::BadRequest {
            message: NO_EMBEDDING.to_owned(),
        });
    };
    let vector = match &record.embedding {
        EmbeddingVector::Floats(values) if is_finite_vector(values) => {
            values.iter().cloned().map(Json::Number).collect()
        }
        EmbeddingVector::Floats(_) | EmbeddingVector::Base64(_) => {
            return Err(MongoDbError::BadRequest {
                message: INVALID_VECTOR.to_owned(),
            });
        }
    };
    let encoded = utf8_percent_encode(vector_store_id, PATH_SEGMENT);
    Ok(SearchRequest {
        url: format!("{api_base}/v1/vector_stores/{encoded}/search"),
        body: request_body(prepared, vector),
    })
}

fn is_finite_vector(values: &[Number]) -> bool {
    !values.is_empty()
        && values
            .iter()
            .all(|value| value.as_f64().is_some_and(f64::is_finite))
}

fn request_body(prepared: &PreparedSearch, vector: Vec<Json>) -> Params {
    let mut body = Params::new();
    body.insert("query".to_owned(), Json::Text(prepared.query_text.clone()));
    body.insert("query_vector".to_owned(), Json::List(vector));
    body.insert(
        "mongodb_database".to_owned(),
        Json::Text(prepared.database.clone()),
    );
    body.insert(
        "mongodb_collection".to_owned(),
        Json::Text(prepared.collection.clone()),
    );
    body.insert(
        "mongodb_embedding_field".to_owned(),
        Json::Text(prepared.embedding_field.clone()),
    );
    body.insert(
        "mongodb_text_field".to_owned(),
        Json::Text(prepared.text_field.clone()),
    );
    body.insert(
        "mongodb_num_candidates".to_owned(),
        Json::Number(Number::from(prepared.num_candidates)),
    );
    body.insert(
        "max_num_results".to_owned(),
        Json::Number(Number::from(prepared.max_num_results)),
    );
    body.insert(
        "timeout_ms".to_owned(),
        Json::Number(Number::from(prepared.timeout_ms)),
    );
    body
}
