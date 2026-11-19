use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::savings_evidence::cache_extraction::{
    UsageObject, extract_cache_creation_tokens, extract_cache_read_tokens,
};

use super::AutorouterFactsError;
use super::counts_as_present;

pub const CACHE_TTL_5M_SECONDS: i64 = 300;
pub const CACHE_TTL_1H_SECONDS: i64 = 3600;

// TurnCacheFacts of autorouter_session_rollup.py: one statement of how a turn's request interacted with the provider's cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnCacheFacts {
    pub covered: bool,
    pub read_tokens: i64,
    pub write_ttl_seconds: Option<i64>,
    pub touched: bool,
}

// turn_cache_facts: absent telemetry reads as touched, the conservative input for the per-model idle clock.
pub fn turn_cache_facts(
    usage_object: Option<&JsonObject>,
) -> Result<TurnCacheFacts, AutorouterFactsError> {
    let covered = usage_object.is_some_and(|usage| !usage.is_empty());
    let usage = decode_usage_object(usage_object)?;
    let read_tokens = extract_cache_read_tokens(usage.as_ref())?;
    let write_ttl_seconds = write_ttl_seconds(usage_object)?;
    Ok(TurnCacheFacts {
        covered,
        read_tokens,
        write_ttl_seconds,
        touched: !covered || read_tokens > 0 || write_ttl_seconds.is_some(),
    })
}

// _write_ttl_seconds: a truthful one-hour split wins over a five-minute one, and an unsplit positive creation count is the five-minute default.
pub fn write_ttl_seconds(
    usage_object: Option<&JsonObject>,
) -> Result<Option<i64>, AutorouterFactsError> {
    let Some(usage) = usage_object.filter(|usage| !usage.is_empty()) else {
        return Ok(None);
    };
    let creation = match usage.get("prompt_tokens_details") {
        Some(Json::Object(details)) => match details.get("cache_creation_token_details") {
            Some(Json::Object(creation)) => Some(creation),
            _ => None,
        },
        _ => None,
    };
    if let Some(creation) = creation {
        if creation
            .get("ephemeral_1h_input_tokens")
            .is_some_and(counts_as_present)
        {
            return Ok(Some(CACHE_TTL_1H_SECONDS));
        }
        if creation
            .get("ephemeral_5m_input_tokens")
            .is_some_and(counts_as_present)
        {
            return Ok(Some(CACHE_TTL_5M_SECONDS));
        }
    }
    let decoded = decode_usage_object(Some(usage))?;
    if extract_cache_creation_tokens(decoded.as_ref())? > 0 {
        return Ok(Some(CACHE_TTL_5M_SECONDS));
    }
    Ok(None)
}

// B1693 owns the cache evidence, so its typed usage object is what both readers below are handed.
fn decode_usage_object(
    usage_object: Option<&JsonObject>,
) -> Result<Option<UsageObject>, AutorouterFactsError> {
    let Some(usage) = usage_object else {
        return Ok(None);
    };
    let encoded = serde_json::to_vec(usage).map_err(AutorouterFactsError::Encode)?;
    let decoded = serde_json::from_slice(&encoded).map_err(AutorouterFactsError::Decode)?;
    Ok(Some(decoded))
}

#[path = "cache/tests.rs"]
mod tests;
