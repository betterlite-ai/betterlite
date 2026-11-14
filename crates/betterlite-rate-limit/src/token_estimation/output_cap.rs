use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use super::RequestData;
use super::call_family::{CallFamily, call_family};
use crate::limiter_records::{
    DEFAULT_MAX_TOKENS_ESTIMATE, RESPONSES_MIN_OUTPUT_TOKENS, TPM_FLOOR_FRACTION,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapEndpoint {
    Generic,
    Decisions,
}

// Python parses int(float(value)) and refuses bool before the numeric check; OverflowError and ValueError mean no cap
#[must_use]
pub fn parse_output_cap_value(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => integer_from_number(number),
        Json::Text(text) => text.trim().parse::<f64>().ok().and_then(truncated_integer),
        Json::Bool(_) | Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

fn integer_from_number(number: &Number) -> Option<i64> {
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }
    if let Some(unsigned) = number.as_u64() {
        return Some(i64::try_from(unsigned).unwrap_or(i64::MAX));
    }
    number.as_f64().and_then(truncated_integer)
}

fn truncated_integer(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let Ok(decimal) = Decimal::try_from(value) else {
        return Some(if value < 0.0_f64 { i64::MIN } else { i64::MAX });
    };
    let truncated = decimal.trunc();
    // Python's int() of a finite float past i64 keeps the value, so a cap that large stays a cap
    let saturated = if truncated.is_sign_negative() {
        i64::MIN
    } else {
        i64::MAX
    };
    Some(truncated.to_i64().unwrap_or(saturated))
}

#[must_use]
pub fn no_max_tokens_output_floor(min_configured_tpm_limit: Option<i64>) -> i64 {
    let baseline = DEFAULT_MAX_TOKENS_ESTIMATE.div_euclid(TPM_FLOOR_FRACTION);
    min_configured_tpm_limit.map_or(baseline, |limit| {
        baseline.min(limit.div_euclid(TPM_FLOOR_FRACTION).max(1))
    })
}

#[must_use]
pub fn output_floor(min_configured_tpm_limit: Option<i64>, call_type: Option<&str>) -> i64 {
    let base = no_max_tokens_output_floor(min_configured_tpm_limit);
    if matches!(call_family(call_type), CallFamily::Responses) {
        return base.max(RESPONSES_MIN_OUTPUT_TOKENS);
    }
    base
}

#[must_use]
pub fn is_embedding_request(data: &RequestData, call_type: Option<&str>) -> bool {
    if matches!(call_family(call_type), CallFamily::Embedding) {
        return true;
    }
    if call_type.is_some() {
        return false;
    }
    data.get("input")
        .is_some_and(|input| !matches!(input, Json::Null))
}

#[must_use]
pub fn explicit_output_cap(data: &RequestData, call_type: Option<&str>) -> Option<i64> {
    match call_family(call_type) {
        CallFamily::Google => {
            let fields = google_config_fields(data)?;
            highest_cap(fields, &["maxOutputTokens", "max_output_tokens"])
        }
        CallFamily::Responses => {
            let cap = parse_output_cap_value(data.get("max_output_tokens")?)?;
            Some(cap.max(RESPONSES_MIN_OUTPUT_TOKENS))
        }
        CallFamily::Embedding => None,
        CallFamily::TextCompletion | CallFamily::Rerank | CallFamily::Other => {
            let names: &[&str] = if call_type.is_some() {
                &["max_tokens", "max_completion_tokens"]
            } else {
                &["max_tokens", "max_completion_tokens", "max_output_tokens"]
            };
            highest_cap(data, names)
        }
    }
}

fn highest_cap(fields: &IndexMap<String, Json>, names: &[&str]) -> Option<i64> {
    let mut highest: Option<i64> = None;
    for name in names {
        if let Some(cap) = fields.get(*name).and_then(parse_output_cap_value) {
            highest = highest.max(Some(cap));
        }
    }
    highest
}

fn google_config_fields(data: &RequestData) -> Option<&IndexMap<String, Json>> {
    let config = data.get("config").or_else(|| data.get("generationConfig"));
    match config {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

#[must_use]
pub fn output_candidate_count(data: &RequestData, call_type: Option<&str>) -> i64 {
    let config_fields = match call_family(call_type) {
        CallFamily::Google => google_config_fields(data),
        CallFamily::Responses
        | CallFamily::Embedding
        | CallFamily::TextCompletion
        | CallFamily::Rerank
        | CallFamily::Other => None,
    };
    let mut count: i64 = 1;
    for value in [data.get("n"), data.get("best_of")] {
        count = count.max(value.and_then(candidate_count_value).unwrap_or(1));
    }
    if let Some(fields) = config_fields {
        for name in ["candidateCount", "candidate_count"] {
            count = count.max(
                fields
                    .get(name)
                    .and_then(candidate_count_value)
                    .unwrap_or(1),
            );
        }
    }
    count
}

fn candidate_count_value(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => integer_from_number(number),
        Json::Text(text) => text.trim().parse::<i64>().ok(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub fn apply_implicit_output_cap(
    data: &mut RequestData,
    min_configured_limit: Option<i64>,
    call_type: Option<&str>,
    configured_output_tokens: Option<i64>,
    endpoint: CapEndpoint,
) {
    let capped_floor = output_floor(min_configured_limit, call_type);
    let baseline_floor = DEFAULT_MAX_TOKENS_ESTIMATE.div_euclid(TPM_FLOOR_FRACTION);
    if capped_floor >= baseline_floor
        || explicit_output_cap(data, call_type).is_some()
        || is_embedding_request(data, call_type)
        || matches!(endpoint, CapEndpoint::Decisions)
    {
        return;
    }
    let effective_cap = capped_floor.max(configured_output_tokens.unwrap_or(0));
    if matches!(call_family(call_type), CallFamily::Google) {
        apply_google_cap(data, effective_cap);
        return;
    }
    let field = if matches!(call_family(call_type), CallFamily::Responses) {
        "max_output_tokens"
    } else {
        "max_tokens"
    };
    // an existing numeric cap would have been an explicit cap above, so only absent or null fields are written
    if matches!(data.get(field), None | Some(Json::Null)) {
        data.insert(field.to_owned(), Json::Number(Number::from(effective_cap)));
    }
}

fn apply_google_cap(data: &mut RequestData, effective_cap: i64) {
    let field = if data.contains_key("config") || !data.contains_key("generationConfig") {
        "config"
    } else {
        "generationConfig"
    };
    let mut fields = match data.get(field) {
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Null) | None => IndexMap::new(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => return,
    };
    fields.insert(
        "maxOutputTokens".to_owned(),
        Json::Number(Number::from(effective_cap)),
    );
    data.insert(field.to_owned(), Json::Object(fields));
}
