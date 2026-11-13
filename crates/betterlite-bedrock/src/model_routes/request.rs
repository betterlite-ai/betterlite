use super::catalog::{RESPONSE_FORMAT_FLAG, TOOLS_REASONING_FLAG, price_map_flag};
use super::{CONVERSE_ONLY_REQUEST_KEYS, ModelRouteSelector, ParamMap, ParamValue, RouteError};

pub fn request_needs_converse(
    selector: &ModelRouteSelector<'_>,
    model: &str,
    request_params: &ParamMap,
) -> Result<bool, RouteError> {
    if request_params.iter().any(|(key, value)| {
        CONVERSE_ONLY_REQUEST_KEYS.contains(&key.as_str()) && !matches!(value, ParamValue::Null)
    }) {
        return Ok(true);
    }
    if selector.request_metadata_owned {
        return Ok(true);
    }
    if response_format_needs_converse(selector, model, request_params.get("response_format"))? {
        return Ok(true);
    }
    let has_tools = holds_any(request_params.get("tools"), "tools")?;
    let has_functions = holds_any(request_params.get("functions"), "functions")?;
    if !(has_tools || has_functions) {
        return Ok(false);
    }
    if price_map_flag(selector, model, TOOLS_REASONING_FLAG)? {
        return Ok(false);
    }
    effort_differs(request_params.get("reasoning_effort"))
}

pub fn response_format_needs_converse(
    selector: &ModelRouteSelector<'_>,
    model: &str,
    response_format: Option<&ParamValue>,
) -> Result<bool, RouteError> {
    let Some(format) = response_format else {
        return Ok(false);
    };
    if matches!(format, ParamValue::Null) {
        return Ok(false);
    }
    let ParamValue::Mapping(fields) = format else {
        return Ok(!price_map_flag(selector, model, RESPONSE_FORMAT_FLAG)?);
    };
    let response_type = match fields.get("type") {
        Some(ParamValue::Text(name)) => Some(name.as_str()),
        Some(_) | None => None,
    };
    if response_type == Some("text") {
        return Ok(false);
    }
    if response_type != Some("json_schema") || !fields.contains_key("json_schema") {
        return Ok(true);
    }
    Ok(!price_map_flag(selector, model, RESPONSE_FORMAT_FLAG)?)
}

const fn holds_any(value: Option<&ParamValue>, field: &'static str) -> Result<bool, RouteError> {
    match value {
        None | Some(ParamValue::Null) => Ok(false),
        Some(ParamValue::List(items)) => Ok(!items.is_empty()),
        Some(_) => Err(RouteError::WrongType { field }),
    }
}

fn effort_differs(value: Option<&ParamValue>) -> Result<bool, RouteError> {
    match value {
        None | Some(ParamValue::Null) => Ok(true),
        Some(ParamValue::Text(effort)) => Ok(effort != "none"),
        Some(_) => Err(RouteError::WrongType {
            field: "reasoning_effort",
        }),
    }
}
