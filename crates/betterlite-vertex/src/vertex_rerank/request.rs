use std::collections::BTreeMap;

use betterlite_logging::analytics::python::repr;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use serde::Serialize;
use serde_json::Number;

use crate::request_labels::request_labels_from_params;

use super::error::VertexRerankError;

const TITLE_WORDS: usize = 3;

#[derive(Debug, Serialize)]
pub struct RankRecord {
    pub id: String,
    pub title: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankRequest {
    pub model: String,
    pub query: String,
    pub records: Vec<RankRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_n: Option<i64>,
    pub ignore_record_details_in_response: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_labels: Option<BTreeMap<String, String>>,
}

#[must_use]
pub fn map_rerank_params(
    non_default_params: &Extra,
    query: &str,
    documents: &[RerankDocument],
    top_n: Option<i64>,
    return_documents: Option<bool>,
) -> Extra {
    let document_values = Json::List(documents.iter().map(RerankDocument::as_json).collect());
    let mut mapped = Extra::new();
    mapped.insert(String::from("query"), Json::Text(query.to_owned()));
    mapped.insert(String::from("documents"), document_values);
    mapped.insert(
        String::from("top_n"),
        top_n.map_or(Json::Null, |count| Json::Number(Number::from(count))),
    );
    mapped.insert(
        String::from("return_documents"),
        return_documents.map_or(Json::Null, Json::Bool),
    );
    mapped.extend(non_default_params.clone());
    mapped
}

pub fn transform_request(
    model: &str,
    params: &Extra,
    litellm_params: Option<&Json>,
) -> Result<RankRequest, VertexRerankError> {
    let Json::Text(query) = required(params, "query")? else {
        return Err(VertexRerankError::FieldType("query", "text"));
    };
    let Json::List(documents) = required(params, "documents")? else {
        return Err(VertexRerankError::FieldType("documents", "a list"));
    };
    let mut records = Vec::with_capacity(documents.len());
    for (index, document) in documents.iter().enumerate() {
        records.push(record(index, document)?);
    }
    Ok(RankRequest {
        model: model.to_owned(),
        query: query.clone(),
        records,
        top_n: top_n(params)?,
        ignore_record_details_in_response: !return_documents(params)?,
        user_labels: request_labels_from_params(litellm_params),
    })
}

fn required<'params>(
    params: &'params Extra,
    field: &'static str,
) -> Result<&'params Json, VertexRerankError> {
    match params.get(field) {
        None | Some(Json::Null) => Err(VertexRerankError::MissingField { field }),
        Some(value) => Ok(value),
    }
}

fn top_n(params: &Extra) -> Result<Option<i64>, VertexRerankError> {
    match params.get("top_n") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) if number.is_i64() => Ok(number.as_i64()),
        Some(_) => Err(VertexRerankError::FieldType("top_n", "a whole number")),
    }
}

// .get("return_documents", True) defaults only an absent key, and the proxy always sends a present None
fn return_documents(params: &Extra) -> Result<bool, VertexRerankError> {
    match params.get("return_documents") {
        None => Ok(true),
        Some(Json::Null) => Ok(false),
        Some(Json::Bool(flag)) => Ok(*flag),
        Some(_) => Err(VertexRerankError::FieldType(
            "return_documents",
            "a boolean",
        )),
    }
}

fn record(index: usize, document: &Json) -> Result<RankRecord, VertexRerankError> {
    let (content, title) = match document {
        Json::Text(text) => (text.clone(), first_words(text)),
        Json::Object(fields) => mapping_content_and_title(index, fields)?,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            return Err(VertexRerankError::DocumentType { index });
        }
    };
    Ok(RankRecord {
        id: index.to_string(),
        title,
        content,
    })
}

// a mapping without text is sent as its Python str(), and a missing title is that content's first words
fn mapping_content_and_title(
    index: usize,
    fields: &Extra,
) -> Result<(String, String), VertexRerankError> {
    let content = match document_text(index, fields, "text")? {
        Some(text) => text.to_owned(),
        None => repr(fields).map_err(|source| VertexRerankError::DocumentText { index, source })?,
    };
    let title =
        document_text(index, fields, "title")?.map_or_else(|| first_words(&content), str::to_owned);
    Ok((content, title))
}

fn document_text<'fields>(
    index: usize,
    fields: &'fields Extra,
    field: &'static str,
) -> Result<Option<&'fields str>, VertexRerankError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(VertexRerankError::DocumentField { index, field }),
    }
}

// Python's str.split() also breaks on the four separator controls that Rust does not call whitespace
fn first_words(content: &str) -> String {
    content
        .split(|character: char| {
            character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
        })
        .filter(|word| !word.is_empty())
        .take(TITLE_WORDS)
        .collect::<Vec<_>>()
        .join(" ")
}
