use betterlite_anthropic::usage::is_anthropic_usage_object;
use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::PrepareError;
use crate::cost::usage::{BillingResponse, BillingUsage, ResponseKind, get_usage_object};

pub(super) struct UsageCounts {
    pub(super) prompt: i64,
    pub(super) completion: i64,
    pub(super) cache_creation: i64,
    pub(super) cache_read: i64,
}

pub(super) struct CountedAttempt {
    pub(super) counts: Result<UsageCounts, PrepareError>,
    pub(super) replaced: Option<Usage>,
}

pub(super) fn counted_usage(usage: &BillingUsage<'_>, installed: Option<&Usage>) -> CountedAttempt {
    if let BillingUsage::TypedRecord(record) = usage {
        return typed_record(record, installed);
    }
    CountedAttempt {
        counts: counts_for(usage),
        replaced: None,
    }
}

fn counts_for(usage: &BillingUsage<'_>) -> Result<UsageCounts, PrepareError> {
    match usage {
        BillingUsage::Absent | BillingUsage::Null => Ok(UsageCounts {
            prompt: 0,
            completion: 0,
            cache_creation: 0,
            cache_read: 0,
        }),
        BillingUsage::Chat(found) => typed_counts(found),
        BillingUsage::Response(found) => {
            converted_counts(ResponseKind::Chat, BillingUsage::Response(found))
        }
        BillingUsage::Transcription(found) => {
            converted_counts(ResponseKind::Chat, BillingUsage::Transcription(found))
        }
        BillingUsage::DecisionsIr(found) => {
            converted_counts(ResponseKind::DecisionsIr, BillingUsage::DecisionsIr(found))
        }
        BillingUsage::DecisionsOpenAi(found) => converted_counts(
            ResponseKind::DecisionsOpenAi,
            BillingUsage::DecisionsOpenAi(found),
        ),
        BillingUsage::Interactions(found) => {
            converted_counts(ResponseKind::Chat, BillingUsage::Interactions(found))
        }
        BillingUsage::Mapping(found) | BillingUsage::TypedRecord(found) => record_counts(found),
    }
}

// A raw record only reaches the generic usage constructor after the recognized shapes are converted
fn converted_counts(
    kind: ResponseKind,
    usage: BillingUsage<'_>,
) -> Result<UsageCounts, PrepareError> {
    let billing = BillingResponse {
        kind,
        usage,
        service_tier: None,
        hidden_params: None,
    };
    get_usage_object(&billing)?.map_or_else(
        || {
            Ok(UsageCounts {
                prompt: 0,
                completion: 0,
                cache_creation: 0,
                cache_read: 0,
            })
        },
        |converted| typed_counts(&converted),
    )
}

fn typed_counts(usage: &Usage) -> Result<UsageCounts, PrepareError> {
    Ok(UsageCounts {
        prompt: usage.prompt_tokens,
        completion: usage.completion_tokens,
        cache_creation: raw_number(&usage.extras, "cache_creation_input_tokens")?,
        cache_read: detail_cache_read(usage.prompt_tokens_details.as_ref())?,
    })
}

fn typed_record(record: &IndexMap<String, Json>, installed: Option<&Usage>) -> CountedAttempt {
    if let Some(usage) = installed {
        return CountedAttempt {
            counts: typed_counts(usage),
            replaced: None,
        };
    }
    let billing = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::TypedRecord(record),
        service_tier: None,
        hidden_params: None,
    };
    match get_usage_object(&billing) {
        Ok(Some(usage)) => CountedAttempt {
            counts: record_counts(record),
            replaced: Some(usage),
        },
        Ok(None) => CountedAttempt {
            counts: record_counts(record),
            replaced: None,
        },
        Err(error) => CountedAttempt {
            counts: Err(PrepareError::Usage(error)),
            replaced: None,
        },
    }
}

fn record_counts(record: &IndexMap<String, Json>) -> Result<UsageCounts, PrepareError> {
    if recognized(record) {
        return converted_counts(ResponseKind::Chat, BillingUsage::Mapping(record));
    }
    let details = match record.get("prompt_tokens_details") {
        None | Some(Json::Null) => None,
        Some(Json::Object(found)) => Some(found),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(PrepareError::Count {
                field: "prompt_tokens_details",
            });
        }
    };
    let cache_read = match details {
        Some(found) if !found.is_empty() => detail_cache_read(Some(found))?,
        Some(_) | None => raw_number(record, "cache_read_input_tokens")?,
    };
    Ok(UsageCounts {
        prompt: raw_number(record, "prompt_tokens")?,
        completion: raw_number(record, "completion_tokens")?,
        cache_creation: raw_number(record, "cache_creation_input_tokens")?,
        cache_read,
    })
}

fn recognized(record: &IndexMap<String, Json>) -> bool {
    is_anthropic_usage_object(&Json::Object(record.clone()))
        || (record.contains_key("input_tokens") && record.contains_key("output_tokens"))
        || (!record.contains_key("prompt_tokens")
            && !record.contains_key("input_tokens")
            && (record.contains_key("total_input_tokens")
                || record.contains_key("total_output_tokens")))
}

fn detail_cache_read(details: Option<&IndexMap<String, Json>>) -> Result<i64, PrepareError> {
    let Some(found) = details.filter(|found| !found.is_empty()) else {
        return Ok(0);
    };
    match found.get("cached_tokens") {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(PrepareError::Count {
            field: "cached_tokens",
        }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(PrepareError::Count {
                field: "cached_tokens",
            })
        }
    }
}

fn raw_number(record: &IndexMap<String, Json>, field: &'static str) -> Result<i64, PrepareError> {
    match record.get(field) {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(PrepareError::Count { field }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(PrepareError::Count { field })
        }
    }
}
