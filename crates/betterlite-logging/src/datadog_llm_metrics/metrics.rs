use betterlite_models::savings_evidence::cache_extraction::{
    UsageObject, extract_cache_creation_tokens, extract_cache_read_tokens,
};
use betterlite_models::savings_evidence::error::SavingsEvidenceError;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::datadog_llm_obs::span_records::LLMMetrics;
use crate::datadog_llm_obs::values::{Object, as_object};
use crate::datadog_llm_privacy::latency::time_to_first_token_seconds;
use crate::fields::payload::StandardLoggingPayload;

use super::reasoning_tokens::reasoning_output_tokens;

#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("the usage object could not be read")]
    UsageObject {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cache token count could not be read")]
    CacheTokens {
        #[source]
        source: SavingsEvidenceError,
    },
}

struct CacheCounts {
    read: Option<f64>,
    write: Option<f64>,
    non_cached: Option<f64>,
}

pub fn span_metrics(
    payload: &StandardLoggingPayload,
    tool_output_tokens: Option<f64>,
) -> Result<LLMMetrics, MetricsError> {
    let usage_fields = payload.metadata.usage_object.as_ref().and_then(as_object);
    let usage = decode_usage(usage_fields)?;
    let cache_read = extract_cache_read_tokens(usage.as_ref())
        .map_err(|source| MetricsError::CacheTokens { source })?;
    let cache_write = extract_cache_creation_tokens(usage.as_ref())
        .map_err(|source| MetricsError::CacheTokens { source })?;
    let cache = cache_counts(payload.prompt_tokens, cache_read, cache_write);
    let reasoning = reasoning_output_tokens(usage_fields);
    let start = payload.start_time.to_string().parse::<f64>().ok();
    let completion_start = payload
        .completion_start_time
        .to_string()
        .parse::<f64>()
        .ok();
    let end = payload.end_time.to_string().parse::<f64>().ok();
    Ok(LLMMetrics {
        input_tokens: Decimal::from(payload.prompt_tokens).to_f64(),
        output_tokens: Decimal::from(payload.completion_tokens).to_f64(),
        total_tokens: Decimal::from(payload.total_tokens).to_f64(),
        time_to_first_token: Some(time_to_first_token_seconds(start, completion_start, end)),
        time_per_output_token: None,
        total_cost: Some(payload.response_cost),
        cache_read_input_tokens: cache.read,
        cache_write_input_tokens: cache.write,
        non_cached_input_tokens: cache.non_cached,
        reasoning_output_tokens: (reasoning != 0.0_f64).then_some(reasoning),
        tool_output_tokens,
    })
}

// LiteLLM treats a usage object that is not a mapping as absent, so only an object is decoded
fn decode_usage(fields: Option<&Object>) -> Result<Option<UsageObject>, MetricsError> {
    let Some(object) = fields else {
        return Ok(None);
    };
    let encoded =
        serde_json::to_value(object).map_err(|source| MetricsError::UsageObject { source })?;
    let decoded =
        serde_json::from_value(encoded).map_err(|source| MetricsError::UsageObject { source })?;
    Ok(Some(decoded))
}

// A zero cache count is absence on the span, while a zero residual is real data and stays
fn cache_counts(prompt_tokens: i64, cache_read: i64, cache_write: i64) -> CacheCounts {
    if cache_read == 0 && cache_write == 0 {
        return CacheCounts {
            read: None,
            write: None,
            non_cached: None,
        };
    }
    let residual = prompt_tokens
        .saturating_sub(cache_read)
        .saturating_sub(cache_write)
        .max(0);
    CacheCounts {
        read: Decimal::from(cache_read)
            .to_f64()
            .filter(|_| cache_read != 0),
        write: Decimal::from(cache_write)
            .to_f64()
            .filter(|_| cache_write != 0),
        non_cached: Decimal::from(residual).to_f64(),
    }
}
