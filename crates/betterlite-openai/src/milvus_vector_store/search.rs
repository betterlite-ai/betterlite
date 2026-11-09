use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;

use super::error::MilvusError;
use super::{MilvusParams, OptionalParams};

const ANN_FIELD: &str = "book_intro_vector";
const SEARCH_PATH: &str = "/v2/vectordb/entities/search";

#[derive(Clone, Copy)]
pub struct Search<'request> {
    pub vector_store_id: &'request str,
    pub query_text: &'request str,
    pub query_vector: &'request [f64],
    pub optional_params: &'request OptionalParams,
    pub api_base: &'request str,
    pub litellm_params: &'request MilvusParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchRequest {
    pub url: String,
    pub body: Json,
    pub query_text: String,
    pub embedding_model: Option<String>,
}

pub fn search_request(search: Search<'_>) -> Result<SearchRequest, MilvusError> {
    Ok(SearchRequest {
        url: format!("{}{SEARCH_PATH}", search.api_base),
        body: search_body(&search)?,
        query_text: search.query_text.to_owned(),
        embedding_model: search.litellm_params.litellm_embedding_model.clone(),
    })
}

fn search_body(search: &Search<'_>) -> Result<Json, MilvusError> {
    // LiteLLM builds the body as a dict literal, so a mapped key replaces its default in place
    let mut body = IndexMap::new();
    body.insert(
        "collectionName".to_owned(),
        Json::Text(search.vector_store_id.to_owned()),
    );
    body.insert(
        "data".to_owned(),
        Json::List(vec![Json::List(vector(search.query_vector)?)]),
    );
    body.insert("annsField".to_owned(), Json::Text(ANN_FIELD.to_owned()));
    for (param, value) in search.optional_params {
        body.insert(param.clone(), value.clone());
    }
    insert_scope(&mut body, search.litellm_params);
    Ok(Json::Object(body))
}

fn insert_scope(body: &mut IndexMap<String, Json>, params: &MilvusParams) {
    let db_name = params
        .milvus_db_name
        .as_ref()
        .filter(|name| !name.is_empty());
    if let Some(name) = db_name {
        body.insert("dbName".to_owned(), Json::Text(name.clone()));
    }
    let partitions = params
        .milvus_partition_names
        .as_ref()
        .filter(|names| !names.is_empty());
    if let Some(names) = partitions {
        let mut entries = Vec::with_capacity(names.len());
        for partition in names {
            entries.push(Json::Text(partition.clone()));
        }
        body.insert("partitionNames".to_owned(), Json::List(entries));
    }
}

fn vector(values: &[f64]) -> Result<Vec<Json>, MilvusError> {
    let mut numbers = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let number = Number::from_f64(*value).ok_or(MilvusError::InvalidVector { index })?;
        numbers.push(Json::Number(number));
    }
    Ok(numbers)
}
