use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::converse_usage::counts::{cache_counts, number_value};
use crate::converse_usage::error::ConverseUsageError;
use crate::converse_usage::wire::ConverseTokenUsage;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCreationTokenDetails {
    #[serde(rename = "ephemeral_5m_input_tokens")]
    pub ephemeral_5m_input_tokens: i64,
    #[serde(rename = "ephemeral_1h_input_tokens")]
    pub ephemeral_1h_input_tokens: i64,
}

// A cacheDetails list only becomes a 5m/1h breakdown when its recognized entries add up to the resolved cache-write count exactly.
pub fn parse_cache_details(
    usage: &ConverseTokenUsage,
) -> Result<Option<CacheCreationTokenDetails>, ConverseUsageError> {
    let Some(details) = usage.cache_details.as_ref() else {
        return Ok(None);
    };
    if empty_cache_details(details) {
        return Ok(None);
    }
    let Json::List(entries) = details else {
        return Err(ConverseUsageError::FieldType {
            field: "cacheDetails",
        });
    };
    let (five_minute_tokens, one_hour_tokens) = bucket_totals(entries)?;
    let total =
        five_minute_tokens
            .checked_add(one_hour_tokens)
            .ok_or(ConverseUsageError::Overflow {
                field: "cacheDetails",
            })?;
    let (_, cache_write) = cache_counts(usage)?;
    if total != cache_write {
        return Ok(None);
    }
    Ok(Some(CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: five_minute_tokens,
        ephemeral_1h_input_tokens: one_hour_tokens,
    }))
}

fn bucket_totals(entries: &[Json]) -> Result<(i64, i64), ConverseUsageError> {
    let mut five_minute_tokens = 0_i64;
    let mut one_hour_tokens = 0_i64;
    for entry in entries {
        let Json::Object(fields) = entry else {
            return Err(ConverseUsageError::FieldType {
                field: "cacheDetails",
            });
        };
        let bucket = match fields.get("ttl") {
            Some(Json::Text(ttl)) if ttl == "5m" => &mut five_minute_tokens,
            Some(Json::Text(ttl)) if ttl == "1h" => &mut one_hour_tokens,
            Some(_) | None => continue,
        };
        // inputTokens is only read for a recognized ttl, so an unknown entry never has to carry it
        let count = entry_tokens(fields)?;
        *bucket = bucket
            .checked_add(count)
            .ok_or(ConverseUsageError::Overflow {
                field: "cacheDetails",
            })?;
    }
    Ok((five_minute_tokens, one_hour_tokens))
}

fn entry_tokens(fields: &IndexMap<String, Json>) -> Result<i64, ConverseUsageError> {
    match fields.get("inputTokens") {
        Some(Json::Number(number)) => number_value(number, "cacheDetails.inputTokens", false),
        Some(Json::Null) | None => Err(ConverseUsageError::Missing {
            field: "cacheDetails.inputTokens",
        }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConverseUsageError::FieldType {
                field: "cacheDetails.inputTokens",
            })
        }
    }
}

// Python's `if not cache_details` check runs before the list iteration, so an empty container is no breakdown at all.
fn empty_cache_details(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(flag) => !flag,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|float| float.to_bits() == 0 || float.to_bits() == 0x8000_0000_0000_0000),
        Json::Text(text) => text.is_empty(),
        Json::List(entries) => entries.is_empty(),
        Json::Object(fields) => fields.is_empty(),
    }
}
