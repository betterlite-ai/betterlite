use std::collections::BTreeMap;

use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult, RerankTokens,
};
use crate::api::vector_store_records::records::score::Score;
use crate::openai_base_config::Headers;

mod tests;

const RERANK_PATH: &str = "rerank";
const JSON_CONTENT_TYPE: &str = "application/json";
const USAGE_FIELDS: [&str; 4] = [
    "search_units",
    "total_tokens",
    "input_tokens",
    "output_tokens",
];

// Python's `return_documents: bool | None = True` argument: Default is the argument the caller omitted
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReturnDocuments {
    #[default]
    Default,
    Null,
    Value(bool),
}

impl ReturnDocuments {
    const fn body_value(self) -> Option<bool> {
        match self {
            Self::Default => Some(true),
            Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}

pub struct RerankCall<'request> {
    pub model: &'request str,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
    pub top_n: Option<i64>,
    pub rank_fields: Option<&'request [String]>,
    pub return_documents: ReturnDocuments,
    pub max_chunks_per_doc: Option<i64>,
    pub api_key: &'request SecretString,
    pub api_base: &'request str,
}

// no Debug: the authorization header carries the resolved key
pub struct RerankHttp {
    pub url: String,
    pub headers: Headers,
    pub body: IndexMap<String, Value>,
}

#[derive(Debug, Error)]
pub enum TogetherRerankError {
    #[error("TogetherAI does not support max_chunks_per_doc")]
    MaxChunksPerDoc,
    #[error("{text}")]
    Response { text: String },
    #[error("No results found in the response={response}")]
    NoResults { response: Value },
    #[error("Missing required fields in the result={result}")]
    MissingResultFields { result: Value },
    #[error("the rerank response usage is not an object: {usage}")]
    InvalidUsage { usage: Value },
    #[error("the rerank response usage field {field} is not an integer: {value}")]
    UsageField { field: String, value: Value },
    #[error("the rerank response usage cannot be read: {source}")]
    UsageDecode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the rerank response document is not an object: {document}")]
    InvalidDocument { document: Value },
    #[error("score cannot be represented as an exact decimal")]
    Score {
        #[source]
        source: DecimalLexemeError,
    },
    #[error("the rerank request cannot be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

pub fn request(call: &RerankCall<'_>) -> Result<RerankHttp, TogetherRerankError> {
    let mut body = IndexMap::new();
    body.insert(String::from("model"), Value::String(call.model.to_owned()));
    body.insert(String::from("query"), Value::String(call.query.to_owned()));
    if let Some(top_n) = call.top_n {
        body.insert(String::from("top_n"), Value::from(top_n));
    }
    body.insert(
        String::from("documents"),
        serde_json::to_value(call.documents)
            .map_err(|source| TogetherRerankError::Encode { source })?,
    );
    if let Some(rank_fields) = call.rank_fields {
        body.insert(
            String::from("rank_fields"),
            serde_json::to_value(rank_fields)
                .map_err(|source| TogetherRerankError::Encode { source })?,
        );
    }
    if let Some(value) = call.return_documents.body_value() {
        body.insert(String::from("return_documents"), Value::Bool(value));
    }
    if call.max_chunks_per_doc.is_some() {
        return Err(TogetherRerankError::MaxChunksPerDoc);
    }
    let headers = Headers::from([
        (String::from("accept"), String::from(JSON_CONTENT_TYPE)),
        (
            String::from("content-type"),
            String::from(JSON_CONTENT_TYPE),
        ),
        (
            String::from("authorization"),
            format!("Bearer {}", call.api_key.expose_secret()),
        ),
    ]);
    Ok(RerankHttp {
        url: format!("{}/{RERANK_PATH}", call.api_base.trim_end_matches('/')),
        headers,
        body,
    })
}

pub fn decode_response(status: u16, text: &str) -> Result<(), TogetherRerankError> {
    if status == 200 {
        return Ok(());
    }
    Err(TogetherRerankError::Response {
        text: text.to_owned(),
    })
}

pub fn transform_response(
    response: &Value,
    next_id: &dyn Fn() -> Uuid,
) -> Result<RerankResponse, TogetherRerankError> {
    let usage = match response.get("usage") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(fields)) => fields.clone(),
        Some(usage) => {
            return Err(TogetherRerankError::InvalidUsage {
                usage: usage.clone(),
            });
        }
    };
    let Some(Value::Array(results)) = response.get("results") else {
        return Err(TogetherRerankError::NoResults {
            response: response.clone(),
        });
    };
    let mut transformed = Vec::with_capacity(results.len());
    for result in results {
        transformed.push(transform_result(result)?);
    }
    let id = response
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map_or_else(|| next_id().to_string(), str::to_owned);
    let (billed_units, tokens) = usage_blocks(usage)?;
    Ok(RerankResponse {
        id: Some(id),
        results: Some(transformed),
        meta: Some(RerankResponseMeta {
            api_version: None,
            billed_units: Some(Some(billed_units)),
            tokens: Some(Some(tokens)),
        }),
        hidden_params: BTreeMap::new(),
    })
}

// Both blocks are filled from the same usage object; each keeps the keys its TypedDict declares
fn usage_blocks(
    usage: Map<String, Value>,
) -> Result<(RerankBilledUnits, RerankTokens), TogetherRerankError> {
    for name in USAGE_FIELDS {
        if let Some(value) = usage.get(name)
            && !value.is_null()
            && value.as_i64().is_none()
        {
            return Err(TogetherRerankError::UsageField {
                field: name.to_owned(),
                value: value.clone(),
            });
        }
    }
    let usage = Value::Object(usage);
    let billed_units = serde_json::from_value(usage.clone())
        .map_err(|source| TogetherRerankError::UsageDecode { source })?;
    let tokens = serde_json::from_value(usage)
        .map_err(|source| TogetherRerankError::UsageDecode { source })?;
    Ok((billed_units, tokens))
}

fn transform_result(result: &Value) -> Result<RerankResponseResult, TogetherRerankError> {
    let (Some(index), Some(Value::Number(score))) = (
        result.get("index").and_then(Value::as_i64),
        result.get("relevance_score"),
    ) else {
        return Err(TogetherRerankError::MissingResultFields {
            result: result.clone(),
        });
    };
    Ok(RerankResponseResult {
        index,
        relevance_score: Score::try_from(score.clone())
            .map_err(|source| TogetherRerankError::Score { source })?,
        document: document(result)?,
    })
}

fn document(result: &Value) -> Result<Option<RerankResponseDocument>, TogetherRerankError> {
    match result.get("document") {
        Some(Value::Object(fields)) if !fields.is_empty() => Ok(Some(RerankResponseDocument {
            text: fields
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        })),
        None | Some(Value::Null | Value::Object(_)) => Ok(None),
        Some(document) => Err(TogetherRerankError::InvalidDocument {
            document: document.clone(),
        }),
    }
}
