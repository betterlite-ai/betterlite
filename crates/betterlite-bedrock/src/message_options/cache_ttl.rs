use crate::bedrock_model_capabilities::CapabilityError;
use crate::bedrock_model_capabilities::catalog_flags::is_claude_4_5_on_bedrock;
use crate::model_routes::{ParamMap, ParamValue};

const EXTENDED_TTLS: [&str; 2] = ["5m", "1h"];

pub fn remove_ttl_from_cache_control(
    request: &mut ParamMap,
    model: Option<&str>,
    model_cost: &ParamMap,
) -> Result<(), CapabilityError> {
    let mut supported = false;
    if let Some(name) = model
        && !name.is_empty()
    {
        supported = is_claude_4_5_on_bedrock(name, model_cost)?;
    }
    if let Some(ParamValue::List(tools)) = request.get_mut("tools") {
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
    if let Some(ParamValue::List(messages)) = request.get_mut("messages") {
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
