use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::HostedVllmRerankError;
use crate::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult, RerankTokens,
};
use crate::api::vector_store_records::records::score::Score;

mod tests;

#[derive(Debug, Deserialize)]
struct RawReply {
    #[serde(default)]
    results: Option<Vec<Value>>,
    #[serde(default)]
    id: Option<String>,
}

pub fn transform_rerank_response(
    status: u16,
    raw: &[u8],
    next_id: &dyn Fn() -> Uuid,
) -> Result<RerankResponse, HostedVllmRerankError> {
    let reply: Value =
        serde_json::from_slice(raw).map_err(|_source| HostedVllmRerankError::Parse {
            text: str::from_utf8(raw)
                .unwrap_or("<body is not UTF-8>")
                .to_owned(),
            status_code: status,
        })?;
    transform_response(&reply, next_id)
}

pub fn transform_response(
    response: &Value,
    next_id: &dyn Fn() -> Uuid,
) -> Result<RerankResponse, HostedVllmRerankError> {
    let decoded: RawReply = serde_json::from_value(response.clone())
        .map_err(|source| HostedVllmRerankError::Shape { source })?;
    let (billed_units, tokens) = usage_blocks(response)?;
    let Some(results) = decoded.results else {
        return Err(HostedVllmRerankError::NoResults {
            response: response.clone(),
        });
    };
    let mut transformed = Vec::with_capacity(results.len());
    for result in &results {
        transformed.push(transform_result(result)?);
    }
    let id = decoded
        .id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| next_id().to_string());
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

// LiteLLM calls response.get("usage", {}).get("total_tokens", 0), which needs an object: absent or null reads zero here, any other value is a typed error
fn usage_blocks(
    response: &Value,
) -> Result<(RerankBilledUnits, RerankTokens), HostedVllmRerankError> {
    let usage = match response.get("usage") {
        None | Some(Value::Null) => None,
        Some(usage) => Some(
            serde_json::from_value::<Map<String, Value>>(usage.clone())
                .map_err(|source| HostedVllmRerankError::Shape { source })?,
        ),
    };
    let total_tokens = match usage.as_ref().and_then(|usage| usage.get("total_tokens")) {
        None => Some(Some(0)),
        Some(Value::Null) => Some(None),
        Some(count) => serde_json::from_value::<i64>(count.clone())
            .map(|total| Some(Some(total)))
            .map_err(|source| HostedVllmRerankError::Shape { source })?,
    };
    Ok((
        RerankBilledUnits {
            search_units: None,
            total_tokens,
        },
        RerankTokens {
            input_tokens: total_tokens,
            output_tokens: None,
        },
    ))
}

fn transform_result(result: &Value) -> Result<RerankResponseResult, HostedVllmRerankError> {
    let (Some(index), Some(Value::Number(score))) = (
        result.get("index").and_then(Value::as_i64),
        result.get("relevance_score"),
    ) else {
        return Err(HostedVllmRerankError::MissingResultFields {
            result: result.clone(),
        });
    };
    Ok(RerankResponseResult {
        index,
        relevance_score: Score::try_from(score.clone())
            .map_err(|source| HostedVllmRerankError::Score { source })?,
        document: document(result),
    })
}

// LiteLLM keeps a non-empty document object and drops every other shape
fn document(result: &Value) -> Option<RerankResponseDocument> {
    match result.get("document") {
        Some(Value::Object(fields)) if !fields.is_empty() => Some(RerankResponseDocument {
            text: fields
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
        _ => None,
    }
}
