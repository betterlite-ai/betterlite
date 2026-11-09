use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde_json::{Map, Value};

use crate::api::vector_store_records::records::score::Score;
use crate::api::vector_store_records::records::vector::{
    VectorStoreResultContent, VectorStoreSearchResponse, VectorStoreSearchResult,
};
use crate::api::vector_store_records::wire::VectorStoreRecordError;

use super::error::{MilvusError, ResponseHeaders, SearchResponseError};

pub const SEARCH_RESULTS_OBJECT: &str = "vector_store.search_results.page";

pub fn decode_milvus_search(
    status: u16,
    headers: &ResponseHeaders,
    body: &[u8],
    query_text: &str,
    text_field: &str,
) -> Result<VectorStoreSearchResponse, MilvusError> {
    let parsed = serde_json::from_slice::<Value>(body)
        .map_err(|source| provider_error(status, headers, SearchResponseError::Body { source }))?;
    let Value::Object(response) = parsed else {
        return Err(provider_error(
            status,
            headers,
            SearchResponseError::NotAnObject,
        ));
    };
    let rows: &[Value] = match response.get("data") {
        None => &[],
        Some(Value::Array(rows)) => rows,
        Some(_) => return Err(provider_error(status, headers, SearchResponseError::Data)),
    };
    let mut results = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let Value::Object(fields) = row else {
            return Err(provider_error(
                status,
                headers,
                SearchResponseError::Row { index },
            ));
        };
        let (text, raw_text) = text_at(fields, text_field);
        let score = score_at(fields, index, status, headers)?;
        results.push(search_result(fields, text, raw_text, score, text_field));
    }
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_RESULTS_OBJECT.to_owned()),
        search_query: Some(query_text.to_owned()),
        data: Some(results),
        extra: BTreeMap::new(),
    })
}

fn text_at(fields: &Map<String, Value>, text_field: &str) -> (Option<String>, Option<Value>) {
    // LiteLLM copies the row value as it is; anything but a string keeps its raw JSON
    match fields.get(text_field) {
        None => (Some(String::new()), None),
        Some(Value::String(text)) => (Some(text.clone()), None),
        Some(other) => (None, Some(other.clone())),
    }
}

fn score_at(
    fields: &Map<String, Value>,
    index: usize,
    status: u16,
    headers: &ResponseHeaders,
) -> Result<Option<Score>, MilvusError> {
    match fields.get("distance") {
        None => Ok(Some(Score(Decimal::new(0, 1)))),
        Some(Value::Number(number)) => match Score::try_from(number.clone()) {
            Ok(score) => Ok(Some(score)),
            Err(source) => Err(MilvusError::ScoreOutOfRange(
                VectorStoreRecordError::ScoreOutOfRange(source),
            )),
        },
        Some(Value::Null) => Ok(None),
        Some(_) => Err(provider_error(
            status,
            headers,
            SearchResponseError::Distance { index },
        )),
    }
}

fn search_result(
    fields: &Map<String, Value>,
    text: Option<String>,
    raw_text: Option<Value>,
    score: Option<Score>,
    text_field: &str,
) -> VectorStoreSearchResult {
    let mut attributes = Map::new();
    for (key, value) in fields {
        if key != "id" && key != "content" && key != "distance" && key != text_field {
            attributes.insert(key.clone(), value.clone());
        }
    }
    // LiteLLM writes file_id, filename and a null score as explicit nulls
    let mut extra = BTreeMap::new();
    extra.insert("file_id".to_owned(), Value::Null);
    extra.insert("filename".to_owned(), Value::Null);
    if score.is_none() {
        extra.insert("score".to_owned(), Value::Null);
    }
    let mut content_extra = BTreeMap::new();
    if let Some(raw) = raw_text {
        content_extra.insert("text".to_owned(), raw);
    }
    let content = VectorStoreResultContent {
        text,
        type_name: Some("text".to_owned()),
        extra: content_extra,
    };
    VectorStoreSearchResult {
        score,
        content: Some(vec![content]),
        file_id: None,
        filename: None,
        attributes: Some(Value::Object(attributes)),
        extra,
    }
}

fn provider_error(
    status: u16,
    headers: &ResponseHeaders,
    source: SearchResponseError,
) -> MilvusError {
    MilvusError::Provider {
        status,
        headers: headers.clone(),
        source,
    }
}
