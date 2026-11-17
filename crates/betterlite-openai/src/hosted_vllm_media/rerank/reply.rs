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

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawReply {
    results: Option<Vec<Value>>,
    id: Option<String>,
}

pub fn transform_rerank_response(
    status: u16,
    raw: &[u8],
    next_id: &dyn Fn() -> Uuid,
) -> Result<RerankResponse, HostedVllmRerankError> {
    let text = str::from_utf8(raw).unwrap_or("<body is not UTF-8>");
    let reply: Value =
        serde_json::from_slice(raw).map_err(|_source| HostedVllmRerankError::Parse {
            text: text.to_owned(),
            status_code: status,
        })?;
    transform_response(&reply, next_id)
}

pub fn transform_response(
    response: &Value,
    next_id: &dyn Fn() -> Uuid,
) -> Result<RerankResponse, HostedVllmRerankError> {
    let decoded: RawReply =
        serde_json::from_value(response.clone()).map_err(HostedVllmRerankError::Shape)?;
    let tokens = reply_tokens(response)?;
    let billed_units = RerankBilledUnits {
        search_units: None,
        total_tokens: tokens.input_tokens,
    };
    let Some(results) = decoded.results else {
        return Err(HostedVllmRerankError::NoResults {
            response: response.clone(),
        });
    };
    let mut transformed = Vec::with_capacity(results.len());
    for result in &results {
        transformed.push(transform_result(result)?);
    }
    let requested_id = decoded.id.filter(|id| !id.is_empty());
    let id = requested_id.unwrap_or_else(|| next_id().to_string());
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
fn reply_tokens(response: &Value) -> Result<RerankTokens, HostedVllmRerankError> {
    let usage = response
        .get("usage")
        .filter(|usage| !usage.is_null())
        .map(|usage| serde_json::from_value::<Map<String, Value>>(usage.clone()))
        .transpose()
        .map_err(HostedVllmRerankError::Shape)?;
    let total_tokens = match usage.as_ref().and_then(|fields| fields.get("total_tokens")) {
        None => Some(Some(0)),
        Some(Value::Null) => Some(None),
        Some(count) => serde_json::from_value::<i64>(count.clone())
            .map(|total| Some(Some(total)))
            .map_err(HostedVllmRerankError::Shape)?,
    };
    Ok(RerankTokens {
        input_tokens: total_tokens,
        output_tokens: None,
    })
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
        relevance_score: Score::try_from(score.clone()).map_err(HostedVllmRerankError::Score)?,
        // LiteLLM keeps a non-empty document object and drops every other shape
        document: match result.get("document") {
            Some(Value::Object(fields)) if !fields.is_empty() => Some(RerankResponseDocument {
                text: fields
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
            _ => None,
        },
    })
}
