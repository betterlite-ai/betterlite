use crate::bedrock_model_capabilities::catalog_flags::is_claude_4_5_on_bedrock;
use crate::model_routes::{ParamMap, ParamValue};

use super::MessageOptionsError;

const EXTENDED_TTLS: [&str; 2] = ["5m", "1h"];

pub fn remove_ttl_from_cache_control(
    request: &mut ParamMap,
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<(), MessageOptionsError> {
    let supported = if let Some(name) = model
        && !name.is_empty()
    {
        is_claude_4_5_on_bedrock(name, model_cost)?
    } else {
        false
    };
    if let Some(tools) = iterable_items(request, "tools")? {
        for tool in tools {
            let ParamValue::Mapping(fields) = tool else {
                continue;
            };
            if let Some(cache_control) = fields.get_mut("cache_control") {
                sanitize_cache_control(cache_control, supported);
            }
        }
    }
    if let Some(ParamValue::List(system)) = request.get_mut("system") {
        sanitize_content(system, supported);
    }
    if let Some(messages) = iterable_items(request, "messages")? {
        for message in messages {
            let ParamValue::Mapping(fields) = message else {
                continue;
            };
            if let Some(ParamValue::List(content)) = fields.get_mut("content") {
                sanitize_content(content, supported);
            }
        }
    }
    Ok(())
}

// Python iterates a present tools or messages value, so null, a bool or a number raises TypeError.
fn iterable_items<'request>(
    request: &'request mut ParamMap,
    field: &'static str,
) -> Result<Option<&'request mut Vec<ParamValue>>, MessageOptionsError> {
    match request.get_mut(field) {
        Some(ParamValue::List(items)) => Ok(Some(items)),
        Some(ParamValue::Null | ParamValue::Bool(_) | ParamValue::Number(_)) => {
            Err(MessageOptionsError::NotIterable { field })
        }
        Some(ParamValue::Text(_) | ParamValue::Mapping(_)) | None => Ok(None),
    }
}

fn sanitize_content(content: &mut [ParamValue], supported: bool) {
    for item in content {
        let ParamValue::Mapping(fields) = item else {
            continue;
        };
        if let Some(cache_control) = fields.get_mut("cache_control") {
            sanitize_cache_control(cache_control, supported);
        }
    }
}

// Python keeps a ttl only when the model supports it and the value is exactly 5m or 1h.
fn sanitize_cache_control(cache_control: &mut ParamValue, supported: bool) {
    let ParamValue::Mapping(fields) = cache_control else {
        return;
    };
    fields.shift_remove("scope");
    let Some(ttl) = fields.get("ttl") else {
        return;
    };
    let keep = supported
        && matches!(ttl, ParamValue::Text(text) if EXTENDED_TTLS.contains(&text.as_str()));
    if !keep {
        fields.shift_remove("ttl");
    }
}
