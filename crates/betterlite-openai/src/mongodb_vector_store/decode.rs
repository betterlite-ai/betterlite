use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Number, Value};

use crate::api::vector_store_records::records::score::Score;
use crate::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResponse, VectorStoreSearchResult,
};

use super::error::MongoDbError;

const INVALID_RESPONSE: &str = "MongoDB sidecar returned an invalid search response. Check the sidecar version and deployment.";
const TEXT_KIND: &str = "text";
pub const SEARCH_RESULTS_OBJECT: &str = "vector_store.search_results.page";

#[derive(Debug, Deserialize)]
struct WireResponse {
    object: String,
    search_query: String,
    data: Vec<WireResult>,
}

#[derive(Debug, Deserialize)]
struct WireResult {
    score: Nullable<Number>,
    content: Vec<WireContent>,
    file_id: Nullable<String>,
    filename: Nullable<String>,
}

#[derive(Debug, Deserialize)]
struct WireContent {
    #[serde(rename = "type")]
    type_name: String,
    text: String,
}

// LiteLLM's strict model requires the key and accepts null, so Nullable separates null from a missing field
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Nullable<T> {
    Value(T),
    Null,
}

pub fn decode_mongodb_search(body: &[u8]) -> Result<VectorStoreSearchResponse, MongoDbError> {
    let response: WireResponse =
        serde_json::from_slice(body).map_err(|_error| MongoDbError::ServiceUnavailable {
            message: INVALID_RESPONSE.to_owned(),
        })?;
    let WireResponse {
        object,
        search_query,
        data,
    } = response;
    if object != SEARCH_RESULTS_OBJECT {
        return Err(MongoDbError::ServiceUnavailable {
            message: INVALID_RESPONSE.to_owned(),
        });
    }
    let mut results = Vec::with_capacity(data.len());
    for row in data {
        results.push(search_result(row)?);
    }
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_RESULTS_OBJECT.to_owned()),
        search_query: Some(search_query),
        data: Some(results),
        extra: BTreeMap::new(),
    })
}

fn search_result(row: WireResult) -> Result<VectorStoreSearchResult, MongoDbError> {
    let decoded_score = nullable_score(row.score)?;
    let file_id = nullable_text(row.file_id);
    let filename = nullable_text(row.filename);
    let mut contents = Vec::with_capacity(row.content.len());
    for WireContent { type_name, text } in row.content {
        if type_name != TEXT_KIND {
            return Err(MongoDbError::ServiceUnavailable {
                message: INVALID_RESPONSE.to_owned(),
            });
        }
        contents.push(VectorStoreResultContent {
            text: Some(text),
            type_name: Some(TEXT_KIND.to_owned()),
            extra: BTreeMap::new(),
        });
    }
    // LiteLLM dumps these keys as explicit nulls, and the record skips a None field
    let mut extra = BTreeMap::new();
    if decoded_score.is_none() {
        extra.insert("score".to_owned(), Value::Null);
    }
    if file_id.is_none() {
        extra.insert("file_id".to_owned(), Value::Null);
    }
    if filename.is_none() {
        extra.insert("filename".to_owned(), Value::Null);
    }
    Ok(VectorStoreSearchResult {
        score: decoded_score,
        content: Some(contents),
        file_id,
        filename,
        attributes: None,
        extra,
    })
}

fn nullable_text(value: Nullable<String>) -> Option<String> {
    match value {
        Nullable::Value(text) => Some(text),
        Nullable::Null => None,
    }
}

fn nullable_score(value: Nullable<Number>) -> Result<Option<Score>, MongoDbError> {
    match value {
        Nullable::Value(number) => Ok(Some(Score::try_from(number).map_err(|_error| {
            MongoDbError::ServiceUnavailable {
                message: INVALID_RESPONSE.to_owned(),
            }
        })?)),
        Nullable::Null => Ok(None),
    }
}
