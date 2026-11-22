use super::TINYFISH_RESULT_CAP;
use super::error::TinyfishSearchError;
use crate::api::search_providers::google_pse::wire::{self, SearchParam};
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::analytics::python;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;
const QUERY_PARAM: &str = "query";
const LOCATION_PARAM: &str = "location";
const COUNTRY_PARAM: &str = "country";
const DOMAIN_PARAM: &str = "search_domain_filter";
const MAX_RESULTS_PARAM: &str = "max_results";
const RESULT_FLOOR: usize = 1;
// max_results is clamped as a truncated float, so the same bounds exist in both number types
const RESULT_FLOOR_FLOAT: f64 = 1.0;
const RESULT_CAP_FLOAT: f64 = 10.0;
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TinyfishBody {
    #[serde(rename = "_tinyfish_params")]
    pub params: IndexMap<String, TinyfishParam>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TinyfishParam {
    Text(String),
    Number(Number),
    Null,
}
// what transform_search_request leaves for transform_search_response: a stored limit or the text to warn about
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CallerLimit {
    Set(usize),
    Ignored(String),
    Absent,
}
pub(super) fn build(query: &SearchQuery, optional_params: &Object) -> (TinyfishBody, CallerLimit) {
    let mut resolved = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(items) => items.join(" "),
    };
    if let Some(domains) = domain_filter(optional_params) {
        let clauses: Vec<String> = domains
            .iter()
            .map(|domain| format!("site:{domain}"))
            .collect();
        resolved = format!("({resolved}) ({})", clauses.join(" OR "));
    }
    let mut params: IndexMap<String, TinyfishParam> = IndexMap::new();
    params.insert(String::from(QUERY_PARAM), TinyfishParam::Text(resolved));
    let country = optional_params
        .get(COUNTRY_PARAM)
        .and_then(JsonValue::as_str);
    if let Some(country) = country {
        params.insert(
            String::from(LOCATION_PARAM),
            TinyfishParam::Text(country.to_owned()),
        );
    }
    let limit = caller_limit(optional_params.get(MAX_RESULTS_PARAM));
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str()) || params.contains_key(name) {
            continue;
        }
        params.insert(name.clone(), param_of(value));
    }
    (TinyfishBody { params }, limit)
}
pub(super) fn query_string(
    params: &IndexMap<String, TinyfishParam>,
) -> Result<String, TinyfishSearchError> {
    let mut encodable: IndexMap<String, SearchParam> = IndexMap::with_capacity(params.len());
    for (name, param) in params {
        let text = match param {
            TinyfishParam::Text(text) => text.clone(),
            TinyfishParam::Number(number) => python::number_text(number),
            TinyfishParam::Null => {
                return Err(TinyfishSearchError::ParamType {
                    field: name.clone(),
                });
            }
        };
        encodable.insert(name.clone(), SearchParam::Text(text));
    }
    Ok(wire::query_string(&encodable, true)?)
}
fn domain_filter(optional_params: &Object) -> Option<Vec<String>> {
    let domains = optional_params.get(DOMAIN_PARAM)?.as_array()?;
    if domains.is_empty() {
        return None;
    }
    domains
        .iter()
        .map(|domain| domain.as_str().map(str::to_owned))
        .collect()
}
fn caller_limit(raw: Option<&JsonValue>) -> CallerLimit {
    let Some(value) = raw else {
        return CallerLimit::Absent;
    };
    match value {
        JsonValue::Bool(_) => CallerLimit::Set(RESULT_FLOOR),
        JsonValue::Number(number) => limit_of_number(number),
        JsonValue::String(text) => limit_of_text(text),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => CallerLimit::Absent,
    }
}
fn limit_of_number(number: &Number) -> CallerLimit {
    let raw = python::number_text(number);
    if raw.contains(['.', 'e', 'E']) {
        return limit_of_float(number.as_f64(), &raw);
    }
    limit_of_text(&raw)
}
// Python's int(text): a numeric string is truncated by int() only when it holds no fractional part
fn limit_of_text(text: &str) -> CallerLimit {
    match parse_caller_int(text) {
        Some(parsed) => CallerLimit::Set(clamped(parsed)),
        None => CallerLimit::Ignored(python::repr(text).unwrap_or_default()),
    }
}
pub(super) fn limit_of_float(value: Option<f64>, raw: &str) -> CallerLimit {
    match value {
        Some(finite) if finite.is_finite() => CallerLimit::Set(clamped_float(finite)),
        Some(_) | None => CallerLimit::Ignored(raw.to_owned()),
    }
}
fn clamped(value: i128) -> usize {
    let Ok(limit) = usize::try_from(value) else {
        return if value.is_positive() {
            TINYFISH_RESULT_CAP
        } else {
            RESULT_FLOOR
        };
    };
    limit.clamp(RESULT_FLOOR, TINYFISH_RESULT_CAP)
}
fn clamped_float(value: f64) -> usize {
    value
        .trunc()
        .max(RESULT_FLOOR_FLOAT)
        .min(RESULT_CAP_FLOAT)
        .to_string()
        .parse::<usize>()
        .unwrap_or(TINYFISH_RESULT_CAP)
}
fn parse_caller_int(text: &str) -> Option<i128> {
    let trimmed = text.trim();
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    if digits.is_empty() {
        return None;
    }
    let mut value = 0_i128;
    let mut digits_seen = false;
    let mut separator_seen = false;
    for character in digits.chars() {
        if let Some(digit) = character.to_digit(10) {
            value = value.saturating_mul(10).saturating_add(i128::from(digit));
            digits_seen = true;
            separator_seen = false;
        } else if character == '_' && digits_seen && !separator_seen {
            separator_seen = true;
        } else {
            return None;
        }
    }
    if separator_seen {
        return None;
    }
    Some(if negative {
        value.saturating_neg()
    } else {
        value
    })
}
fn param_of(value: &JsonValue) -> TinyfishParam {
    match value {
        JsonValue::Bool(flag) => {
            TinyfishParam::Text(String::from(if *flag { "true" } else { "false" }))
        }
        JsonValue::Number(number) => TinyfishParam::Number(number.clone()),
        JsonValue::String(text) => TinyfishParam::Text(text.clone()),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            TinyfishParam::Text(serde_json::to_string(value).unwrap_or_default())
        }
        JsonValue::Null => TinyfishParam::Null,
    }
}
