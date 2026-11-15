use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult, RerankTokens,
};
use crate::api::vector_store_records::records::score::Score;
use crate::huggingface_rerank::error::{
    HuggingfaceRerankError, MAX_RESPONSE_BYTES, get_error_class,
};
use crate::huggingface_rerank::params::Params;
use crate::huggingface_rerank::{RequestIdSource, TextTokenCounter};

const API_VERSION: &str = "1.0";
const DEFAULT_ERROR_STATUS: u16 = 500;
const INPUT_TOKENS_PER_CHAR: u64 = 4;
const OUTPUT_TOKENS_PER_ITEM: u64 = 10;
const SEARCH_UNITS: i64 = 1;
const TEXT_FIELD: &str = "text";

pub struct ResponseRequest<'request> {
    pub model: &'request str,
    pub status_code: Option<u16>,
    pub body: &'request [u8],
    pub request_data: &'request Params,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RerankResponseItem {
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub score: Option<Score>,
    #[serde(default)]
    pub text: Option<String>,
}

struct RequestData {
    query: String,
    texts: Vec<RerankDocument>,
    return_documents: bool,
}

enum Field<'params> {
    Absent,
    Text(&'params str),
    Bool(bool),
    List(&'params [Json]),
    Other,
}

fn field<'params>(params: &'params Params, name: &'static str) -> Field<'params> {
    match params.get(name) {
        None | Some(Json::Null) => Field::Absent,
        Some(Json::Text(text)) => Field::Text(text),
        Some(Json::Bool(value)) => Field::Bool(*value),
        Some(Json::List(items)) => Field::List(items),
        Some(Json::Number(_) | Json::Object(_)) => Field::Other,
    }
}

impl RequestData {
    fn read(params: &Params) -> Result<Self, HuggingfaceRerankError> {
        let query = match field(params, "query") {
            Field::Absent => String::new(),
            Field::Text(text) => text.to_owned(),
            Field::Bool(_) | Field::List(_) | Field::Other => {
                return Err(HuggingfaceRerankError::RequestField { field: "query" });
            }
        };
        let mut return_documents = false;
        for name in ["return_documents", "return_text"] {
            match field(params, name) {
                Field::Absent => {}
                Field::Bool(value) => return_documents |= value,
                Field::Text(_) | Field::List(_) | Field::Other => {
                    return Err(HuggingfaceRerankError::RequestField { field: name });
                }
            }
        }
        let texts = match field(params, "texts") {
            Field::Absent => Vec::new(),
            Field::List(items) => items
                .iter()
                .map(document_from)
                .collect::<Result<Vec<_>, _>>()?,
            Field::Text(_) | Field::Bool(_) | Field::Other => {
                return Err(HuggingfaceRerankError::RequestField { field: "texts" });
            }
        };
        Ok(Self {
            query,
            texts,
            return_documents,
        })
    }
}

pub fn decode_rerank_response(
    body: &[u8],
    status_code: Option<u16>,
) -> Result<Vec<RerankResponseItem>, HuggingfaceRerankError> {
    let value = parse_body(body, status_code)?;
    let Value::Array(entries) = &value else {
        return Err(HuggingfaceRerankError::ResponseShape);
    };
    project_items(entries)
}

pub fn transform_rerank_response(
    request: &ResponseRequest<'_>,
    counter: &dyn TextTokenCounter,
    ids: &dyn RequestIdSource,
) -> Result<RerankResponse, HuggingfaceRerankError> {
    let value = parse_body(request.body, request.status_code)?;
    let Value::Array(entries) = &value else {
        return Err(HuggingfaceRerankError::ResponseShape);
    };
    let items = project_items(entries)?;
    let output_text = value.to_string();
    let data = RequestData::read(request.request_data)?;
    let (output_tokens, input_tokens) =
        count_tokens(request.model, &output_text, &items, &data, counter)?;
    Ok(RerankResponse {
        id: Some(ids.new_id()),
        results: Some(results(&items, &data)?),
        meta: Some(meta(output_tokens, input_tokens)?),
        hidden_params: BTreeMap::new(),
    })
}

fn parse_body(body: &[u8], status_code: Option<u16>) -> Result<Value, HuggingfaceRerankError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(HuggingfaceRerankError::ResponseTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let status = status_code.unwrap_or(DEFAULT_ERROR_STATUS);
    serde_json::from_slice(body).map_err(|_source| {
        // invalid UTF-8 never parses as JSON, so the substitute is the message
        let text = str::from_utf8(body).unwrap_or("the response body is not UTF-8");
        HuggingfaceRerankError::Huggingface(get_error_class(text, status))
    })
}

fn project_items(entries: &[Value]) -> Result<Vec<RerankResponseItem>, HuggingfaceRerankError> {
    entries
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            RerankResponseItem::deserialize(entry)
                .map_err(|source| HuggingfaceRerankError::ResponseItem { position, source })
        })
        .collect()
}

fn document_from(value: &Json) -> Result<RerankDocument, HuggingfaceRerankError> {
    match value {
        Json::Text(text) => Ok(RerankDocument::Text(text.clone())),
        Json::Object(fields) => Ok(RerankDocument::Object(fields.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Err(HuggingfaceRerankError::RequestField { field: "texts" })
        }
    }
}

fn count_tokens(
    model: &str,
    output_text: &str,
    items: &[RerankResponseItem],
    data: &RequestData,
    counter: &dyn TextTokenCounter,
) -> Result<(u64, u64), HuggingfaceRerankError> {
    let Ok(output_tokens) = counter.count(model, output_text) else {
        return Ok((fallback_output(items)?, fallback_input(&data.query)?));
    };
    let mut input_text = data.query.clone();
    input_text.push(' ');
    for (position, document) in data.texts.iter().enumerate() {
        if let Some(text) = document_text(document, position)? {
            input_text.push_str(text);
            input_text.push(' ');
        }
    }
    match counter.count(model, &input_text) {
        Ok(input_tokens) => Ok((output_tokens, input_tokens)),
        Err(_source) => Ok((fallback_output(items)?, fallback_input(&input_text)?)),
    }
}

fn document_text(
    document: &RerankDocument,
    position: usize,
) -> Result<Option<&str>, HuggingfaceRerankError> {
    match document {
        RerankDocument::Text(text) => Ok(Some(text)),
        RerankDocument::Object(fields) => match fields.get(TEXT_FIELD) {
            None | Some(Json::Null) => Ok(None),
            Some(Json::Text(text)) => Ok(Some(text)),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                Err(HuggingfaceRerankError::DocumentText { position })
            }
        },
    }
}

fn fallback_output(items: &[RerankResponseItem]) -> Result<u64, HuggingfaceRerankError> {
    if items.is_empty() {
        return Ok(OUTPUT_TOKENS_PER_ITEM);
    }
    u64::try_from(items.len())
        .ok()
        .and_then(|count| count.checked_mul(OUTPUT_TOKENS_PER_ITEM))
        .ok_or(HuggingfaceRerankError::TokenOverflow)
}

fn fallback_input(text: &str) -> Result<u64, HuggingfaceRerankError> {
    u64::try_from(text.chars().count())
        .ok()
        .and_then(|count| count.checked_mul(INPUT_TOKENS_PER_CHAR))
        .ok_or(HuggingfaceRerankError::TokenOverflow)
}

fn results(
    items: &[RerankResponseItem],
    data: &RequestData,
) -> Result<Vec<RerankResponseResult>, HuggingfaceRerankError> {
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        let Some(index) = item.index else {
            continue;
        };
        let Some(score) = item.score.clone() else {
            continue;
        };
        let document = if data.return_documents {
            result_document(item, data, index)?
        } else {
            None
        };
        results.push(RerankResponseResult {
            index,
            relevance_score: score,
            document,
        });
    }
    Ok(results)
}

fn result_document(
    item: &RerankResponseItem,
    data: &RequestData,
    index: i64,
) -> Result<Option<RerankResponseDocument>, HuggingfaceRerankError> {
    if let Some(text) = item.text.as_deref().filter(|text| !text.is_empty()) {
        return Ok(Some(RerankResponseDocument {
            text: text.to_owned(),
        }));
    }
    let Ok(position) = usize::try_from(index) else {
        return Ok(None);
    };
    let Some(document) = data.texts.get(position) else {
        return Ok(None);
    };
    Ok(
        document_text(document, position)?.map(|text| RerankResponseDocument {
            text: text.to_owned(),
        }),
    )
}

fn meta(
    output_tokens: u64,
    input_tokens: u64,
) -> Result<RerankResponseMeta, HuggingfaceRerankError> {
    let input =
        i64::try_from(input_tokens).map_err(|_error| HuggingfaceRerankError::TokenOverflow)?;
    let output =
        i64::try_from(output_tokens).map_err(|_error| HuggingfaceRerankError::TokenOverflow)?;
    let api_version = IndexMap::from([("version".to_owned(), Json::Text(API_VERSION.to_owned()))]);
    Ok(RerankResponseMeta {
        api_version: Some(Some(api_version)),
        billed_units: Some(Some(RerankBilledUnits {
            search_units: Some(Some(SEARCH_UNITS)),
            total_tokens: None,
        })),
        tokens: Some(Some(RerankTokens {
            input_tokens: Some(Some(input)),
            output_tokens: Some(Some(output)),
        })),
    })
}
