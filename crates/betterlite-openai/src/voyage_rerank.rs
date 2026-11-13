use std::str;

use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use crate::api::rerank_contracts::records::RerankDocument;

pub const SUPPORTED_COHERE_RERANK_PARAMS: [&str; 4] =
    ["query", "documents", "top_n", "return_documents"];
pub const DEFAULT_COMPLETE_URL: &str = "https://api.voyageai.com/v1/rerank";

#[derive(Debug, Error)]
pub enum VoyageRerankError {
    #[error(
        "Voyage AI API key is required. Set via `api_key` parameter or `VOYAGE_API_KEY` env var."
    )]
    MissingApiKey,
    #[error("the Voyage rerank endpoint answered HTTP {status}")]
    Status { status: u16 },
    #[error("the Voyage rerank response is not valid JSON: {message}")]
    Decode {
        status: u16,
        message: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("No results found in the response")]
    NoResults,
    #[error("the Voyage rerank response field `{field}` is missing or has the wrong type")]
    ResponseField { field: &'static str },
}

#[derive(Debug, Error)]
pub enum VoyageRerankCostError {
    #[error("the Voyage rerank cost exceeds the representable decimal range")]
    Overflow,
}

// LiteLLM reads only these two declared controls, so rank_fields and the two limits never reach the body
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoyageRerankControls {
    pub top_n: Option<i64>,
    pub return_documents: Option<bool>,
    pub rank_fields: Option<Vec<String>>,
    pub max_chunks_per_doc: Option<i64>,
    pub max_tokens_per_doc: Option<i64>,
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoyageRerankBody<'request> {
    pub model: &'request str,
    pub query: &'request str,
    pub documents: &'request [RerankDocument],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_documents: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoyageRerankResponse {
    pub id: String,
    pub results: Vec<VoyageRerankResult>,
    pub meta: VoyageRerankMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoyageRerankResult {
    pub index: i64,
    pub relevance_score: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoyageRerankMeta {
    pub billed_units: VoyageRerankBilledUnits,
    pub tokens: VoyageRerankTokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VoyageRerankBilledUnits {
    pub total_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VoyageRerankTokens {
    pub input_tokens: Option<i64>,
    pub output_tokens: i64,
}

// LiteLLM logs the post call after the status check and before it parses the body
pub trait RerankLogging {
    fn post_call(&mut self, original_response: &[u8]);
}

// an explicit key never falls back to the environment, and VOYAGE_AI_TOKEN is not one of rerank's two aliases
pub fn api_key<F>(explicit: Option<&str>, lookup: F) -> Result<String, VoyageRerankError>
where
    F: Fn(&str) -> Option<String>,
{
    let resolved = match explicit {
        Some(key) => Some(key.to_owned()),
        None => lookup("VOYAGE_API_KEY")
            .filter(|value| !value.is_empty())
            .or_else(|| lookup("VOYAGE_AI_API_KEY")),
    };
    resolved.ok_or(VoyageRerankError::MissingApiKey)
}

#[must_use]
pub fn headers(_supplied: Map<String, Value>, api_key: &str) -> Map<String, Value> {
    Map::from_iter([
        (
            "Authorization".to_owned(),
            Value::String(format!("Bearer {api_key}")),
        ),
        (
            "content-type".to_owned(),
            Value::String("application/json".to_owned()),
        ),
    ])
}

#[must_use]
pub fn endpoint(base: Option<&str>) -> String {
    let Some(base) = base else {
        return DEFAULT_COMPLETE_URL.to_owned();
    };
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1/rerank") {
        return trimmed.to_owned();
    }
    if trimmed.ends_with("/v1") {
        return format!("{trimmed}/rerank");
    }
    format!("{trimmed}/v1/rerank")
}

#[must_use]
pub fn request<'request>(
    model: &'request str,
    query: &'request str,
    documents: &'request [RerankDocument],
    controls: &VoyageRerankControls,
) -> VoyageRerankBody<'request> {
    VoyageRerankBody {
        model,
        query,
        documents,
        top_k: controls.top_n,
        return_documents: controls.return_documents,
    }
}

pub fn decode_voyage_rerank(
    bytes: &[u8],
    status: u16,
    uuid: Uuid,
    logging: &mut dyn RerankLogging,
) -> Result<VoyageRerankResponse, VoyageRerankError> {
    if status != 200 {
        return Err(VoyageRerankError::Status { status });
    }
    logging.post_call(bytes);
    let parsed: Json =
        serde_json::from_slice(bytes).map_err(|source| VoyageRerankError::Decode {
            status,
            message: str::from_utf8(bytes).map_or_else(|_| source.to_string(), str::to_owned),
            source,
        })?;
    let Json::Object(root) = parsed else {
        return Err(VoyageRerankError::ResponseField { field: "response" });
    };
    let total_tokens = total_tokens(&root)?;
    Ok(VoyageRerankResponse {
        id: response_id(&root, uuid)?,
        results: results(&root)?,
        meta: VoyageRerankMeta {
            billed_units: VoyageRerankBilledUnits { total_tokens },
            tokens: VoyageRerankTokens {
                input_tokens: total_tokens,
                output_tokens: 0,
            },
        },
    })
}

pub fn calculate_rerank_cost(
    input_cost_per_token: Option<Decimal>,
    billed_units: Option<&VoyageRerankBilledUnits>,
) -> Result<(Decimal, Decimal), VoyageRerankCostError> {
    let (Some(rate), Some(units)) = (input_cost_per_token, billed_units) else {
        return Ok((Decimal::ZERO, Decimal::ZERO));
    };
    let Some(total_tokens) = units.total_tokens else {
        return Ok((Decimal::ZERO, Decimal::ZERO));
    };
    let cost = Decimal::from(total_tokens)
        .checked_mul(rate)
        .ok_or(VoyageRerankCostError::Overflow)?;
    Ok((cost, Decimal::ZERO))
}

fn response_id(root: &IndexMap<String, Json>, uuid: Uuid) -> Result<String, VoyageRerankError> {
    let Some(value) = root.get("id").filter(|value| is_truthy(*value)) else {
        return Ok(uuid.to_string());
    };
    let Json::Text(text) = value else {
        return Err(VoyageRerankError::ResponseField { field: "id" });
    };
    Ok(text.clone())
}

fn results(root: &IndexMap<String, Json>) -> Result<Vec<VoyageRerankResult>, VoyageRerankError> {
    match root.get("data") {
        None | Some(Json::Null) => Err(VoyageRerankError::NoResults),
        Some(Json::List(items)) => items.iter().map(result).collect(),
        Some(_) => Err(VoyageRerankError::ResponseField { field: "data" }),
    }
}

fn result(item: &Json) -> Result<VoyageRerankResult, VoyageRerankError> {
    let Json::Object(fields) = item else {
        return Err(VoyageRerankError::ResponseField { field: "data[]" });
    };
    let Some(Json::Number(number)) = fields.get("index") else {
        return Err(VoyageRerankError::ResponseField { field: "index" });
    };
    let Some(index) = number.as_i64() else {
        return Err(VoyageRerankError::ResponseField { field: "index" });
    };
    let Some(Json::Number(number)) = fields.get("relevance_score") else {
        return Err(VoyageRerankError::ResponseField {
            field: "relevance_score",
        });
    };
    let relevance_score = parse_decimal_lexeme(&number.to_string())
        .map(Decimal::normalize)
        .map_err(|_| VoyageRerankError::ResponseField {
            field: "relevance_score",
        })?;
    let document = fields.get("document").map(|value| match value {
        Json::Text(text) => Json::Object(IndexMap::from([(
            "text".to_owned(),
            Json::Text(text.clone()),
        )])),
        other => other.clone(),
    });
    Ok(VoyageRerankResult {
        index,
        relevance_score,
        document,
    })
}

fn total_tokens(root: &IndexMap<String, Json>) -> Result<Option<i64>, VoyageRerankError> {
    let usage = match root.get("usage") {
        None | Some(Json::Null) => return Ok(Some(0)),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VoyageRerankError::ResponseField { field: "usage" });
        }
    };
    match usage.get("total_tokens") {
        None => Ok(Some(0)),
        Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) if number.is_i64() => Ok(number.as_i64()),
        Some(_) => Err(VoyageRerankError::ResponseField {
            field: "usage.total_tokens",
        }),
    }
}

fn is_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

#[path = "voyage_rerank/tests.rs"]
mod tests;
