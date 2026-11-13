use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::usage_sum::Usage;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::gemini_candidates::candidate::Candidate;
use crate::gemini_response::decode::decode_generate_content;
use crate::gemini_usage::error::UsageError;
use crate::gemini_usage::metadata::UsageMetadata;
use crate::vertex_batch_records::error::BatchRecordsError;

pub const VERTEX_AI_PROVIDER: &str = "vertex_ai";

#[derive(Clone, Debug)]
pub struct NativeVertexBatchRowStats {
    pub usage: Usage,
    pub total_tokens: i64,
    pub model: Option<String>,
    pub prompt_cost: Decimal,
    pub completion_cost: Decimal,
}

pub struct RowCostRequest<'request> {
    pub usage: &'request Usage,
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub model_info: Option<&'request ModelInfo>,
}

pub trait RowCostCalculator {
    fn calculate(&self, request: &RowCostRequest<'_>) -> Result<GenericCostResult, TokenCostError>;
}

#[must_use]
pub fn is_native_batch_row(row: &Json) -> bool {
    let Json::Object(fields) = row else {
        return false;
    };
    matches!(fields.get("request"), Some(Json::Object(_)))
}

pub fn native_vertex_batch_row_stats<U, C>(
    row: &Json,
    model_name: Option<&str>,
    model_info: Option<&ModelInfo>,
    convert_usage: U,
    cost_calculator: &C,
) -> Result<Option<NativeVertexBatchRowStats>, BatchRecordsError>
where
    U: Fn(Option<&UsageMetadata>, &[Json]) -> Result<Usage, UsageError>,
    C: RowCostCalculator,
{
    let Json::Object(fields) = row else {
        return Ok(None);
    };
    let Some(Json::Object(response)) = fields.get("response") else {
        return Ok(None);
    };
    let Some(usage) = native_vertex_row_usage(response, &convert_usage)? else {
        return Ok(None);
    };
    let total_tokens = total_tokens(&usage)?;
    let model = row_model(response, model_name);
    if model.is_none() {
        tracing::warn!(
            "a vertex_ai batch output row has no modelVersion and the batch has no deployment model, so it is billed at $0 and the rest of the batch is still billed"
        );
    }
    let priced = model
        .as_deref()
        .and_then(|priced_model| row_cost(priced_model, &usage, model_info, cost_calculator));
    let (prompt_cost, completion_cost) = priced.map_or((Decimal::ZERO, Decimal::ZERO), |cost| {
        (cost.prompt_cost, cost.completion_cost)
    });
    Ok(Some(NativeVertexBatchRowStats {
        usage,
        total_tokens,
        model,
        prompt_cost,
        completion_cost,
    }))
}

// LiteLLM keeps one unpriceable row from aborting the batch's cost accounting
fn row_cost<C>(
    model: &str,
    usage: &Usage,
    model_info: Option<&ModelInfo>,
    cost_calculator: &C,
) -> Option<GenericCostResult>
where
    C: RowCostCalculator,
{
    match cost_calculator.calculate(&RowCostRequest {
        usage,
        model,
        custom_llm_provider: VERTEX_AI_PROVIDER,
        model_info,
    }) {
        Ok(cost) => Some(cost),
        Err(error) => {
            tracing::warn!(
                model = %model,
                error = %error,
                "a vertex_ai batch output row could not be costed, so it is billed at $0 and the rest of the batch is still billed"
            );
            None
        }
    }
}

pub fn native_vertex_row_usage<U>(
    response: &IndexMap<String, Json>,
    convert_usage: &U,
) -> Result<Option<Usage>, BatchRecordsError>
where
    U: Fn(Option<&UsageMetadata>, &[Json]) -> Result<Usage, UsageError>,
{
    if !response.contains_key("usageMetadata") {
        return Ok(embedding_usage(response));
    }
    if has_null_envelope_field(response) {
        return Ok(None);
    }
    let bytes = serde_json::to_vec(response).map_err(BatchRecordsError::Decode)?;
    let decoded = match decode_generate_content(&bytes) {
        Ok(decoded) => decoded,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "a vertex_ai batch row response is not a GenerateContentResponse, so the row is skipped"
            );
            return Ok(None);
        }
    };
    let Some(metadata) = decoded.usage_metadata else {
        return Ok(None);
    };
    let candidates = candidate_metadata(decoded.candidates.unwrap_or_default());
    convert_usage(Some(&metadata), &candidates)
        .map(Some)
        .map_err(BatchRecordsError::Usage)
}

#[must_use]
pub fn embedding_prompt_token_count(response: &IndexMap<String, Json>) -> i64 {
    match response.get("usageMetadata") {
        Some(Json::Object(metadata)) => int_field(metadata, "promptTokenCount"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => int_field(response, "tokenCount"),
    }
}

// LiteLLM bills a row without usageMetadata from its documented embedding tokenCount
fn embedding_usage(response: &IndexMap<String, Json>) -> Option<Usage> {
    if !matches!(response.get("embedding"), Some(Json::Object(_))) {
        return None;
    }
    let prompt_tokens = embedding_prompt_token_count(response);
    Some(Usage {
        prompt_tokens,
        completion_tokens: 0,
        total_tokens: prompt_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    })
}

// LiteLLM's TypeAdapter rejects a null where it declares candidates, promptFeedback or responseId
fn has_null_envelope_field(response: &IndexMap<String, Json>) -> bool {
    for key in ["candidates", "promptFeedback", "responseId"] {
        if response.get(key) == Some(&Json::Null) {
            return true;
        }
    }
    false
}

fn candidate_metadata(candidates: Vec<Candidate>) -> Vec<Json> {
    let mut metadata = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        metadata.push(Json::Object(candidate.metadata));
    }
    metadata
}

fn total_tokens(usage: &Usage) -> Result<i64, BatchRecordsError> {
    if usage.total_tokens != 0 {
        return Ok(usage.total_tokens);
    }
    usage
        .prompt_tokens
        .checked_add(usage.completion_tokens)
        .ok_or(UsageError::Overflow {
            field: "total_tokens",
        })
        .map_err(BatchRecordsError::Usage)
}

fn row_model(response: &IndexMap<String, Json>, model_name: Option<&str>) -> Option<String> {
    let deployment_model = model_name.filter(|name| !name.is_empty() && !name.contains('*'));
    let fallback = match response.get("modelVersion") {
        Some(Json::Text(version)) => Some(version.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => model_name.map(str::to_owned),
    };
    deployment_model.map(str::to_owned).or(fallback)
}

fn int_field(mapping: &IndexMap<String, Json>, key: &str) -> i64 {
    match mapping.get(key) {
        Some(Json::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(Json::Text(text))
            if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            text.parse().unwrap_or_default()
        }
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
}
