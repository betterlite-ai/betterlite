use std::collections::BTreeSet;

use betterlite_context::bedrock_model::split_bedrock_region_path;
use indexmap::IndexSet;

use crate::model_routes::catalog::openai_gpt_version;
use crate::model_routes::{ModelRouteSelector, ParamMap, ParamValue, RouteError};

pub const REFUSED_BY_FAMILY: &[(&str, &[&str])] = &[
    ("openai.gpt-oss", &["logit_bias"]),
    ("xai.", &["frequency_penalty", "presence_penalty"]),
];
const REFUSED_WHILE_REASONING: [&str; 6] = [
    "temperature",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
    "logprobs",
    "top_logprobs",
];
pub const REFUSED_EFFORTS_BY_FAMILY: &[(&str, &[&str])] = &[("xai.", &["none"])];
const EFFORT_KEY: &str = "reasoning_effort";

#[must_use]
pub fn union_by_family(model: &str, table: &[(&str, &[&str])]) -> BTreeSet<String> {
    let (_, model_id) = split_bedrock_region_path(model);
    let mut values = BTreeSet::new();
    for (family, names) in table {
        if model_id.contains(*family) {
            for name in *names {
                values.insert(String::from(*name));
            }
        }
    }
    values
}

#[must_use]
pub fn without_params(params: &ParamMap, dropped: &BTreeSet<String>) -> ParamMap {
    params
        .iter()
        .filter(|(key, _value)| !dropped.contains(*key))
        .map(|(key, value)| (String::from(key.as_str()), value.clone()))
        .collect()
}

#[must_use]
pub fn without_refused_effort(model: &str, params: &ParamMap) -> ParamMap {
    let refused = union_by_family(model, REFUSED_EFFORTS_BY_FAMILY);
    let refused_effort = matches!(
        params.get(EFFORT_KEY),
        Some(ParamValue::Text(effort)) if refused.contains(effort)
    );
    if !refused_effort {
        return params.clone();
    }
    without_params(params, &BTreeSet::from([String::from(EFFORT_KEY)]))
}

#[must_use]
pub fn non_string_reasoning_effort(params: &ParamMap) -> BTreeSet<String> {
    match params.get(EFFORT_KEY) {
        None | Some(ParamValue::Null | ParamValue::Text(_)) => BTreeSet::new(),
        Some(
            ParamValue::Bool(_)
            | ParamValue::Number(_)
            | ParamValue::List(_)
            | ParamValue::Mapping(_),
        ) => BTreeSet::from([String::from(EFFORT_KEY)]),
    }
}

pub fn params_refused_while_reasoning(
    model: &str,
    params: &ParamMap,
) -> Result<BTreeSet<String>, RouteError> {
    let explicit_none = matches!(
        params.get(EFFORT_KEY),
        Some(ParamValue::Text(effort)) if effort == "none"
    );
    if explicit_none || !is_openai_gpt(model)? {
        return Ok(BTreeSet::new());
    }
    Ok(REFUSED_WHILE_REASONING
        .into_iter()
        .filter(|name| params.contains_key(*name))
        .map(String::from)
        .collect())
}

fn is_openai_gpt(model: &str) -> Result<bool, RouteError> {
    let price_map = ParamMap::new();
    let converse_models = IndexSet::new();
    let selector = ModelRouteSelector::new(&price_map, &converse_models, false)?;
    Ok(openai_gpt_version(&selector, model)?.is_some())
}
