use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::basic::VectorStoreStatus;
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreFileCounts, VectorStoreResultContent,
    VectorStoreSearchResponse, VectorStoreSearchResult,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use thiserror::Error;

use super::super::error::{ProviderError, VertexRagStoreError};
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_fine_tuning::records::timestamp::created_at;

const SEARCH_PAGE_OBJECT: &str = "vector_store.search_results.page";
const CREATE_OBJECT: &str = "vector_store";
const UNKNOWN_DOCUMENT: &str = "Unknown Document";
const NO_CONTEXTS: &[Json] = &[];

pub struct Reply<'reply> {
    pub status: u16,
    pub headers: &'reply BTreeMap<String, String>,
    pub body: &'reply [u8],
}

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("the vertex RAG payload is not valid JSON: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("the vertex RAG payload field {field} must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the vertex RAG payload entry {index} of {field} must be {expected}")]
    Entry {
        field: &'static str,
        index: usize,
        expected: &'static str,
    },
}

// LiteLLM's transform_search_vector_store_response, with the search query it reads back from the logged call
pub fn transform_search_response(
    reply: &Reply<'_>,
    logging: &BTreeMap<String, Json>,
) -> Result<VectorStoreSearchResponse, VertexRagStoreError> {
    let search_query = logged_query(logging)?;
    decode_vertex_rag_payload(reply.body, search_query)
        .map_err(|source| provider_error(reply, source))
}

// LiteLLM's transform_create_vector_store_response, keeping every null the corpus reply carries
pub fn transform_create_response(
    reply: &Reply<'_>,
    local: &LocalTimeContext,
) -> Result<VectorStoreCreateResponse, VertexRagStoreError> {
    decode_vertex_rag_corpus(reply.body, local).map_err(|source| provider_error(reply, source))
}

pub fn decode_vertex_rag_payload(
    body: &[u8],
    search_query: &str,
) -> Result<VectorStoreSearchResponse, PayloadError> {
    let parsed = serde_json::from_slice::<Json>(body)?;
    let Json::Object(payload) = parsed else {
        return Err(PayloadError::Field {
            field: "the reply",
            expected: "an object",
        });
    };
    let items = context_items(&payload)?;
    let mut data = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        data.push(search_result(index, item)?);
    }
    Ok(VectorStoreSearchResponse {
        object: Some(SEARCH_PAGE_OBJECT.to_owned()),
        search_query: Some(search_query.to_owned()),
        data: Some(data),
        extra: BTreeMap::new(),
    })
}

fn context_items<'payload>(
    payload: &'payload IndexMap<String, Json>,
) -> Result<&'payload [Json], PayloadError> {
    let group = match payload.get("contexts") {
        None | Some(Json::Null) => None,
        Some(Json::Object(group)) => Some(group),
        Some(_) => {
            return Err(PayloadError::Field {
                field: "contexts",
                expected: "an object",
            });
        }
    };
    match group.and_then(|group| group.get("contexts")) {
        None | Some(Json::Null) => Ok(NO_CONTEXTS),
        Some(Json::List(items)) => Ok(items.as_slice()),
        Some(_) => Err(PayloadError::Field {
            field: "contexts.contexts",
            expected: "an array",
        }),
    }
}

fn search_result(index: usize, item: &Json) -> Result<VectorStoreSearchResult, PayloadError> {
    let Json::Object(fields) = item else {
        return Err(PayloadError::Entry {
            field: "contexts.contexts",
            index,
            expected: "an object",
        });
    };
    let source = field_or_new(fields, "sourceUri");
    let display = field_or_new(fields, "sourceDisplayName");
    let page_span = match fields.get("pageSpan") {
        Some(value) => value.clone(),
        None => Json::Object(IndexMap::new()),
    };
    let attributes = serde_json::to_value(attributes(&source, &display, page_span))?;
    let mut result = VectorStoreSearchResult {
        score: None,
        content: Some(vec![content(fields)?]),
        file_id: None,
        filename: None,
        attributes: Some(attributes),
        extra: BTreeMap::new(),
    };
    let file_id = if truthy(&source) { &source } else { &display };
    if let Some(raw) = text_value(&mut result.file_id, file_id) {
        result
            .extra
            .insert("file_id".to_owned(), serde_json::to_value(&raw)?);
    }
    let filename = if truthy(&display) {
        display.clone()
    } else {
        Json::Text(UNKNOWN_DOCUMENT.to_owned())
    };
    if let Some(raw) = text_value(&mut result.filename, &filename) {
        result
            .extra
            .insert("filename".to_owned(), serde_json::to_value(&raw)?);
    }
    match fields.get("score") {
        None => {
            result.score = Some(Score(Decimal::new(0, 1)));
        }
        Some(Json::Number(number)) => match Score::try_from(number.clone()) {
            Ok(score) => result.score = Some(score),
            Err(_error) => {
                result
                    .extra
                    .insert("score".to_owned(), serde_json::to_value(number)?);
            }
        },
        Some(other) => {
            result
                .extra
                .insert("score".to_owned(), serde_json::to_value(other)?);
        }
    }
    Ok(result)
}

fn field_or_new(fields: &IndexMap<String, Json>, name: &str) -> Json {
    match fields.get(name) {
        Some(value) => value.clone(),
        None => Json::Text(String::new()),
    }
}

fn content(fields: &IndexMap<String, Json>) -> Result<VectorStoreResultContent, PayloadError> {
    let mut content = VectorStoreResultContent {
        text: Some(String::new()),
        type_name: Some("text".to_owned()),
        extra: BTreeMap::new(),
    };
    match fields.get("text") {
        None => {}
        Some(Json::Text(text)) => content.text = Some(text.clone()),
        Some(other) => {
            content.text = None;
            content
                .extra
                .insert("text".to_owned(), serde_json::to_value(other)?);
        }
    }
    Ok(content)
}

fn attributes(source: &Json, display: &Json, page_span: Json) -> Json {
    let mut attributes = IndexMap::new();
    if truthy(source) {
        attributes.insert("sourceUri".to_owned(), source.clone());
    }
    if truthy(display) {
        attributes.insert("sourceDisplayName".to_owned(), display.clone());
    }
    if truthy(&page_span) {
        attributes.insert("pageSpan".to_owned(), page_span);
    }
    Json::Object(attributes)
}

fn text_value(field: &mut Option<String>, value: &Json) -> Option<Json> {
    match value {
        Json::Text(text) => {
            *field = Some(text.clone());
            None
        }
        other => Some(other.clone()),
    }
}

// Python's truthiness decides which source names a result and which metadata keys reach attributes
fn truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => !number_is_zero(number),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

fn number_is_zero(number: &Number) -> bool {
    betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme(&number.to_string())
        .is_ok_and(|value| value.is_zero())
}

fn logged_query(logging: &BTreeMap<String, Json>) -> Result<&str, VertexRagStoreError> {
    match logging.get("query") {
        None | Some(Json::Null) => Ok(""),
        Some(Json::Text(text)) => Ok(text),
        Some(_) => Err(VertexRagStoreError::ParamType {
            field: "query",
            expected: "a string",
        }),
    }
}

fn provider_error(reply: &Reply<'_>, error: PayloadError) -> VertexRagStoreError {
    let headers = reply
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    VertexRagStoreError::Provider(ProviderError {
        status_code: reply.status,
        message: error.to_string(),
        headers,
    })
}

pub fn decode_vertex_rag_corpus(
    body: &[u8],
    local: &LocalTimeContext,
) -> Result<VectorStoreCreateResponse, PayloadError> {
    let parsed = serde_json::from_slice::<Json>(body)?;
    let Json::Object(payload) = parsed else {
        return Err(PayloadError::Field {
            field: "the reply",
            expected: "an object",
        });
    };
    let corpus_name = text_field(&payload, "name")?;
    let id = corpus_name
        .rsplit_once('/')
        .map_or(corpus_name.as_str(), |(_head, tail)| tail)
        .to_owned();
    let mut extra = BTreeMap::new();
    let mut name = Some(String::new());
    match payload.get("display_name") {
        None => {}
        Some(Json::Text(text)) => name = Some(text.clone()),
        Some(other) => {
            name = None;
            extra.insert("name".to_owned(), serde_json::to_value(other)?);
        }
    }
    let created_at = match payload.get("createTime") {
        Some(value) => create_time(value, local),
        None => 0,
    };
    let metadata = labels(payload.get("labels"))?;
    for field in ["expires_after", "expires_at", "last_active_at"] {
        extra.insert(field.to_owned(), serde_json::to_value(Json::Null)?);
    }
    Ok(VectorStoreCreateResponse {
        id: Some(id),
        object: Some(CREATE_OBJECT.to_owned()),
        created_at: Some(created_at),
        name,
        bytes: Some(0),
        file_counts: Some(VectorStoreFileCounts {
            in_progress: Some(0),
            completed: Some(0),
            failed: Some(0),
            cancelled: Some(0),
            total: Some(0),
            extra: BTreeMap::new(),
        }),
        status: Some(VectorStoreStatus::Completed),
        expires_after: None,
        expires_at: None,
        last_active_at: None,
        metadata: Some(metadata),
        extra,
    })
}

fn text_field(
    payload: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<String, PayloadError> {
    match payload.get(field) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(PayloadError::Field {
            field,
            expected: "a string",
        }),
    }
}

fn create_time(value: &Json, local: &LocalTimeContext) -> i64 {
    match value {
        Json::Bool(flag) => {
            if *flag {
                1
            } else {
                0
            }
        }
        Json::Number(number) => match number.as_i64() {
            Some(value) => value,
            None => number
                .as_u64()
                .map_or(0, |value| i64::try_from(value).unwrap_or(0)),
        },
        Json::Text(text) => created_at(Some(text), local),
        Json::Null | Json::List(_) | Json::Object(_) => 0,
    }
}

fn labels(value: Option<&Json>) -> Result<BTreeMap<String, String>, PayloadError> {
    let Some(Json::Object(labels)) = value else {
        return Ok(BTreeMap::new());
    };
    let mut metadata = BTreeMap::new();
    for (name, value) in labels {
        let Json::Text(text) = value else {
            return Err(PayloadError::Field {
                field: "labels",
                expected: "an object of strings",
            });
        };
        metadata.insert(name.clone(), text.clone());
    }
    Ok(metadata)
}
