use std::collections::BTreeSet;

use betterlite_models::decisions::Json;

use crate::datadog::tagging::normalize_value;
use crate::datadog_llm_obs::constants::{COST_DIMENSIONS, ROUTER_DIMENSIONS};
use crate::datadog_llm_obs::values::Object;
use crate::fields::payload::StandardLoggingPayload;

// LiteLLM tags the span with the user, key, model group and router dimensions, normalized and never empty
#[must_use]
pub fn cost_dimension_tags(
    payload: &StandardLoggingPayload,
    router_fields: &Object,
) -> Vec<String> {
    let metadata = &payload.metadata;
    let mut dimensions: Vec<(&str, Option<String>)> = vec![
        ("user", metadata.user_api_key_user_id.clone()),
        ("key_alias", metadata.user_api_key_alias.clone()),
        ("model_group", payload.model_group.clone()),
    ];
    for dimension in ROUTER_DIMENSIONS {
        let value = router_fields.get(dimension).and_then(dimension_text);
        dimensions.push((dimension, value));
    }
    dimensions
        .into_iter()
        .filter_map(|(name, value)| tag(name, value.as_deref()))
        .collect()
}

// LiteLLM intersects the span's tags with the declared dimensions, so only the ones that carry a value are listed
#[must_use]
pub fn declared_cost_tags(span_tags: &[String]) -> Vec<String> {
    let mut present = BTreeSet::new();
    for tag in span_tags {
        let Some((key, value)) = tag.split_once(':') else {
            continue;
        };
        if !key.is_empty() && !value.is_empty() {
            present.insert(key);
        }
    }
    let mut declared = Vec::new();
    for dimension in COST_DIMENSIONS {
        if present.contains(dimension) {
            declared.push(dimension.to_owned());
        }
    }
    declared
}

fn dimension_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Bool(flag) => Some(flag.to_string()),
        Json::Number(number) => Some(number.to_string()),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

fn tag(name: &str, value: Option<&str>) -> Option<String> {
    let text = value?;
    let normalized = normalize_value(text);
    if normalized.is_empty() {
        return None;
    }
    Some(format!("{name}:{normalized}"))
}

mod tests;
