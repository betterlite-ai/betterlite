use betterlite_context::bedrock_model::{split_bedrock_region_path, strip_bedrock_routing_prefix};

use super::{DEFAULT_SINCE, ModelRouteSelector, ParamValue, RouteError};

pub const CHAT_COMPLETIONS_ENDPOINT: &str = "/v1/chat/completions";
pub const RESPONSE_FORMAT_FLAG: &str = "supports_bedrock_runtime_chat_completions_response_format";
pub const TOOLS_REASONING_FLAG: &str =
    "supports_bedrock_runtime_chat_completions_tools_with_reasoning";
const ENDPOINTS_FIELD: &str = "supported_endpoints";

pub fn price_map_flag(
    selector: &ModelRouteSelector<'_>,
    model: &str,
    flag: &str,
) -> Result<bool, RouteError> {
    let mut found = false;
    for entry in price_entries(selector, model)? {
        if let Some(price_entry) = entry
            && flag_is_literal_true(price_entry, flag)
        {
            found = true;
        }
    }
    Ok(found)
}

pub fn chat_completions_is_default(
    selector: &ModelRouteSelector<'_>,
    model: &str,
) -> Result<bool, RouteError> {
    let Some(version) = openai_gpt_version(selector, model)? else {
        return Ok(false);
    };
    if version < DEFAULT_SINCE {
        return Ok(false);
    }
    let mut found = false;
    for entry in price_entries(selector, model)? {
        if let Some(price_entry) = entry
            && entry_lists_endpoint(price_entry, CHAT_COMPLETIONS_ENDPOINT)
        {
            found = true;
        }
    }
    Ok(found)
}

pub fn openai_gpt_version(
    selector: &ModelRouteSelector<'_>,
    model: &str,
) -> Result<Option<(u16, u16)>, RouteError> {
    let Some(captures) = selector.gpt_version_pattern.captures(model)? else {
        return Ok(None);
    };
    let major = captures
        .get(2)
        .and_then(|digits| digits.as_str().parse::<u16>().ok());
    let minor = captures
        .get(3)
        .and_then(|digits| digits.as_str().parse::<u16>().ok());
    Ok(major.map(|major_value| (major_value, minor.unwrap_or(0))))
}

fn price_entries<'entry>(
    selector: &'entry ModelRouteSelector<'_>,
    model: &str,
) -> Result<Vec<Option<&'entry ParamValue>>, RouteError> {
    let (_, region_model) = split_bedrock_region_path(model);
    let keys = [
        model.to_owned(),
        strip_bedrock_routing_prefix(model),
        region_model,
    ];
    let mut entries = Vec::with_capacity(keys.len());
    for key in keys {
        entries.push(price_entry(selector, &key)?);
    }
    Ok(entries)
}

fn price_entry<'entry>(
    selector: &'entry ModelRouteSelector<'_>,
    key: &str,
) -> Result<Option<&'entry ParamValue>, RouteError> {
    match selector.price_map.get(key) {
        None | Some(ParamValue::Null) => Ok(None),
        Some(entry @ ParamValue::Mapping(_)) => Ok(Some(entry)),
        Some(_) => Err(RouteError::MalformedPriceEntry {
            key: key.to_owned(),
        }),
    }
}

fn flag_is_literal_true(entry: &ParamValue, flag: &str) -> bool {
    match entry {
        ParamValue::Mapping(fields) => matches!(fields.get(flag), Some(ParamValue::Bool(true))),
        ParamValue::Null
        | ParamValue::Bool(_)
        | ParamValue::Number(_)
        | ParamValue::Text(_)
        | ParamValue::List(_) => false,
    }
}

fn entry_lists_endpoint(entry: &ParamValue, endpoint: &str) -> bool {
    let ParamValue::Mapping(fields) = entry else {
        return false;
    };
    match fields.get(ENDPOINTS_FIELD) {
        Some(ParamValue::List(endpoints)) => endpoints
            .iter()
            .any(|item| matches!(item, ParamValue::Text(text) if text == endpoint)),
        Some(_) | None => false,
    }
}
