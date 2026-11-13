use betterlite_context::bedrock_model::get_bedrock_base_model;
use fancy_regex::Regex;
use indexmap::IndexMap;

use crate::model_routes::{ParamMap, ParamValue};

use super::CapabilityError;

const CACHE_POINT_FLAG: &str = "supports_prompt_caching";
const EXTENDED_CACHE_FIELD: &str = "cache_creation_input_token_cost_above_1hr";
const OPENAI_FAMILY_PATTERN: &str = r"(^|[./])openai\.";
const PARALLEL_TOOL_FLAG: &str = "supports_parallel_tool_use_config";

pub fn accepts_cache_points(
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<bool, CapabilityError> {
    let Some(name) = model else {
        return Ok(true);
    };
    if Regex::new(OPENAI_FAMILY_PATTERN)?.is_match(name)? {
        return Ok(false);
    }
    let entries: Vec<&ParamValue> = [name.to_owned(), get_bedrock_base_model(name)]
        .into_iter()
        .filter_map(|candidate| model_cost.get(&candidate))
        .filter(|entry| !matches!(**entry, ParamValue::Null))
        .collect();
    if entries.is_empty() {
        return Ok(true);
    }
    for entry in entries {
        let ParamValue::Mapping(fields) = entry else {
            return Err(CapabilityError::MalformedEntry {
                model: name.to_owned(),
            });
        };
        if matches!(fields.get(CACHE_POINT_FLAG), Some(ParamValue::Bool(true))) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn supports_parallel_tools(
    model: &str,
    model_cost: &ParamMap,
) -> Result<bool, CapabilityError> {
    any_entry(model, model_cost, |fields| {
        matches!(fields.get(PARALLEL_TOOL_FLAG), Some(ParamValue::Bool(true)))
    })
}

pub fn is_claude_4_5_on_bedrock(
    model: &str,
    model_cost: &ParamMap,
) -> Result<bool, CapabilityError> {
    let base = get_bedrock_base_model(model);
    for candidate in [model, base.as_str()] {
        let entry = present_entry(model_cost.get(candidate), candidate)?;
        if entry.is_some_and(|fields| {
            fields
                .get(EXTENDED_CACHE_FIELD)
                .is_some_and(|value| !matches!(value, ParamValue::Null))
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Python's any(): a truthy entry stops early, and a malformed one raises only when reached
fn any_entry<Found>(
    model: &str,
    model_cost: &ParamMap,
    found: Found,
) -> Result<bool, CapabilityError>
where
    Found: Fn(&IndexMap<String, ParamValue>) -> bool,
{
    for candidate in [model.to_owned(), get_bedrock_base_model(model)] {
        let Some(fields) = present_entry(model_cost.get(&candidate), &candidate)? else {
            continue;
        };
        if found(fields) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Python's `(entry or {})`, where a present non-object fails on the .get that follows
fn present_entry<'map>(
    entry: Option<&'map ParamValue>,
    model: &str,
) -> Result<Option<&'map IndexMap<String, ParamValue>>, CapabilityError> {
    match entry {
        None => Ok(None),
        Some(value) if !counts_as_present(value) => Ok(None),
        Some(ParamValue::Mapping(fields)) => Ok(Some(fields)),
        Some(_) => Err(CapabilityError::MalformedEntry {
            model: model.to_owned(),
        }),
    }
}

fn counts_as_present(value: &ParamValue) -> bool {
    match value {
        ParamValue::Null => false,
        ParamValue::Bool(flag) => *flag,
        ParamValue::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        ParamValue::Text(text) => !text.is_empty(),
        ParamValue::List(items) => !items.is_empty(),
        ParamValue::Mapping(fields) => !fields.is_empty(),
    }
}
