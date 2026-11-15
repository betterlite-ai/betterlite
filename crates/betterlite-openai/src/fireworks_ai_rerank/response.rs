use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult, RerankTokens,
};
use crate::api::vector_store_records::records::score::Score;
use crate::fireworks_ai_common::error::FireworksError;
use crate::openai_base_config::Headers;

use super::error::FireworksRerankError;

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub trait IdSource: Send + Sync {
    fn new_uuid(&self) -> Uuid;
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UuidIds;

impl IdSource for UuidIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::new_v4()
    }
}
#[derive(Debug, Default, Deserialize)]
pub struct RawUsage {
    #[serde(default, rename = "total_tokens")]
    total: Option<i64>,
    #[serde(default, rename = "prompt_tokens")]
    prompt: Option<i64>,
    #[serde(default, rename = "completion_tokens")]
    completion: Option<i64>,
}
#[derive(Debug, Deserialize)]
pub struct RawResult {
    #[serde(default)]
    index: Option<serde_json::Number>,
    #[serde(default)]
    relevance_score: Option<serde_json::Number>,
    #[serde(default)]
    document: Option<Json>,
}
#[derive(Debug, Default, Deserialize)]
pub struct RawRerankResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    usage: Option<RawUsage>,
    #[serde(default)]
    data: Option<Vec<RawResult>>,
    #[serde(default)]
    results: Option<Vec<RawResult>>,
}
pub fn decode_rerank_response(bytes: &[u8]) -> Result<RawRerankResponse, FireworksRerankError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(FireworksRerankError::ResponseTooLarge {
            size: bytes.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| FireworksRerankError::ResponseDecode { source })
}
fn project_result(result: RawResult) -> Result<RerankResponseResult, FireworksRerankError> {
    let raw = result
        .index
        .ok_or(FireworksRerankError::MissingResultField { field: "index" })?;
    let index = raw.as_i64().ok_or(FireworksRerankError::IndexShape)?;
    let relevance = result
        .relevance_score
        .ok_or(FireworksRerankError::MissingResultField {
            field: "relevance_score",
        })?;
    let relevance_score = Score::try_from(relevance)
        .map_err(|source| FireworksRerankError::RelevanceScore { source })?;
    let document = match result.document.as_ref() {
        Some(Json::Text(text)) if !text.is_empty() => {
            Some(RerankResponseDocument { text: text.clone() })
        }
        Some(Json::Object(fields)) => match fields.get("text") {
            Some(Json::Text(text)) if !text.is_empty() => {
                Some(RerankResponseDocument { text: text.clone() })
            }
            _ => None,
        },
        _ => None,
    };
    Ok(RerankResponseResult {
        index,
        relevance_score,
        document,
    })
}
pub fn transform_rerank_response(
    status_code: u16,
    headers: &Headers,
    body: &[u8],
    ids: &dyn IdSource,
) -> Result<RerankResponse, FireworksRerankError> {
    let reply = decode_rerank_response(body).map_err(|error| match error {
        FireworksRerankError::ResponseDecode { source } => {
            FireworksRerankError::Fireworks(FireworksError {
                status_code,
                message: format!("Failed to parse response: {source}"),
                headers: Some(headers.clone()),
            })
        }
        other @ (FireworksRerankError::MissingApiKey
        | FireworksRerankError::MissingQuery
        | FireworksRerankError::MissingDocuments
        | FireworksRerankError::ResponseTooLarge { .. }
        | FireworksRerankError::NoResults
        | FireworksRerankError::MissingResultField { .. }
        | FireworksRerankError::IndexShape
        | FireworksRerankError::RelevanceScore { .. }
        | FireworksRerankError::Fireworks(_)) => other,
    })?;
    let usage = reply.usage.unwrap_or_default();
    let results = match reply.data.filter(|data| !data.is_empty()) {
        Some(data) => data,
        None => reply.results.ok_or(FireworksRerankError::NoResults)?,
    };
    let mut projected = Vec::with_capacity(results.len());
    for result in results {
        projected.push(project_result(result)?);
    }
    let id = reply
        .id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| ids.new_uuid().to_string());
    let meta = RerankResponseMeta {
        api_version: None,
        billed_units: Some(Some(RerankBilledUnits {
            search_units: Some(Some(usage.total.unwrap_or_default())),
            total_tokens: None,
        })),
        tokens: Some(Some(RerankTokens {
            input_tokens: Some(Some(usage.prompt.unwrap_or_default())),
            output_tokens: Some(Some(usage.completion.unwrap_or_default())),
        })),
    };
    Ok(RerankResponse {
        id: Some(id),
        results: Some(projected),
        meta: Some(meta),
        hidden_params: BTreeMap::<String, Json>::new(),
    })
}
