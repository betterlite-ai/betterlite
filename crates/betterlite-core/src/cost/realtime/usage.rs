use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{Usage, UsageNumber};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::api::responses_usage::{UsageInput, normalize_usage};

use super::error::RealtimeCostError;

// Ports the realtime half of _transform_response_api_usage_to_chat_usage: an absent or null usage bills zero tokens.
pub(super) fn aggregate_usage(value: Option<&Json>) -> Result<Usage, RealtimeCostError> {
    match value {
        None | Some(Json::Null) => Ok(Usage::default()),
        Some(Json::Object(fields)) => aggregate_mapping(fields),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(RealtimeCostError::Shape {
                field: "usage",
                expected: "an object or null",
            })
        }
    }
}

pub(super) fn aggregate_mapping(
    fields: &IndexMap<String, Json>,
) -> Result<Usage, RealtimeCostError> {
    let raw = serde_json::to_value(fields).map_err(|source| RealtimeCostError::Json { source })?;
    let converted = normalize_usage(UsageInput::Mapping(&raw))?;
    let mut numeric_extras = BTreeMap::new();
    if let Some(cost) = converted.cost {
        numeric_extras.insert("cost".to_owned(), UsageNumber::Decimal(cost));
    }
    for (name, extension) in &converted.extensions {
        if name.starts_with('_') {
            continue;
        }
        if let Some(integer) = extension.as_i64() {
            numeric_extras.insert(name.clone(), UsageNumber::Integer(integer));
            continue;
        }
        if let Some(flag) = extension.as_bool() {
            numeric_extras.insert(name.clone(), UsageNumber::Boolean(flag));
            continue;
        }
        let Some(float) = extension.as_f64() else {
            continue;
        };
        let number = Decimal::try_from(float).map_err(|source| RealtimeCostError::Rate {
            field: "usage extension",
            source,
        })?;
        numeric_extras.insert(name.clone(), UsageNumber::Decimal(number));
    }
    Ok(Usage {
        prompt_tokens: converted.prompt_tokens,
        completion_tokens: converted.completion_tokens,
        total_tokens: converted.total_tokens,
        prompt_tokens_details: converted.prompt_tokens_details,
        completion_tokens_details: converted.completion_tokens_details,
        numeric_extras,
    })
}
