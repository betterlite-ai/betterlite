use super::JsonObject;
use serde_json::Value;

pub(super) fn valid_shape(body: &JsonObject, plan: bool) -> bool {
    let common = [
        "model",
        "max_tokens",
        "stream",
        "temperature",
        "top_p",
        "top_k",
        "stop_sequences",
        "metadata",
        "messages",
        "system",
        "tools",
    ];
    let extras = [
        "cache_control",
        "thinking",
        "tool_choice",
        "output_config",
        "speed",
        "service_tier",
    ];
    if !body.keys().all(|key| {
        common.contains(&key.as_str())
            || (plan && extras.contains(&key.as_str()))
            || (!plan && false)
    }) {
        return false;
    }
    if !valid_options(body, plan)
        || !body.get("messages").is_some_and(|value| {
            value.as_array().is_some_and(|items| {
                !items.is_empty() && items.iter().all(|item| valid_message(item, plan))
            })
        })
    {
        return false;
    }
    if let Some(system) = body.get("system") {
        let valid = match system {
            Value::String(text) => nonempty(text),
            Value::Array(items) => items.iter().all(|item| {
                if plan {
                    return valid_block(item, true, true);
                }
                let Some(block) = item.as_object() else {
                    return false;
                };
                fields(block, &["type", "text"])
                    && block.get("type").and_then(Value::as_str) == Some("text")
                    && block.get("text").is_some_and(Value::is_string)
            }),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => false,
        };
        if !valid {
            return false;
        }
    }
    if let Some(tools) = body.get("tools") {
        if !tools.as_array().is_some_and(|items| {
            items.iter().all(|value| {
                let Some(tool) = value.as_object() else {
                    return false;
                };
                let allowed = if plan {
                    &[
                        "name",
                        "description",
                        "input_schema",
                        "type",
                        "cache_control",
                    ][..]
                } else {
                    &["name", "description", "input_schema", "type"][..]
                };
                fields(tool, allowed)
                    && tool
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(nonempty)
                    && tool.get("description").is_none_or(Value::is_string)
                    && tool.get("input_schema").is_some_and(Value::is_object)
                    && tool
                        .get("type")
                        .is_none_or(|item| item.as_str() == Some("custom"))
                    && (!plan || valid_marker(tool))
            })
        }) {
            return false;
        }
    }
    if let Some(control) = body.get("cache_control") {
        if !plan || cache_ttl(control).is_none() {
            return false;
        }
    }
    true
}

pub(super) fn valid_options(body: &JsonObject, plan: bool) -> bool {
    let common = body.get("model").is_none_or(Value::is_string)
        && body.get("max_tokens").is_none_or(integer)
        && body.get("stream").is_none_or(Value::is_boolean)
        && body.get("temperature").is_none_or(number)
        && body.get("top_p").is_none_or(number)
        && body.get("top_k").is_none_or(integer)
        && body.get("stop_sequences").is_none_or(|value| {
            value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        })
        && body.get("metadata").is_none_or(Value::is_object);
    let plan_options = body.get("thinking").is_none_or(Value::is_object)
        && body.get("tool_choice").is_none_or(Value::is_object)
        && body.get("output_config").is_none_or(Value::is_object)
        && body.get("speed").is_none_or(|value| {
            value
                .as_str()
                .is_some_and(|text| text == "fast" || text == "standard")
        })
        && body.get("service_tier").is_none_or(|value| {
            value
                .as_str()
                .is_some_and(|text| text == "auto" || text == "standard_only")
        });
    common && (!plan || plan_options)
}

fn valid_message(value: &Value, plan: bool) -> bool {
    let Some(message) = value.as_object() else {
        return false;
    };
    let roles = if plan {
        ["user", "assistant", "system"].as_slice()
    } else {
        ["user", "assistant"].as_slice()
    };
    message.keys().all(|key| key == "role" || key == "content")
        && message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| roles.contains(&role))
        && message.get("content").is_some_and(|content| {
            content.as_str().is_some_and(nonempty)
                || content
                    .as_array()
                    .is_some_and(|items| items.iter().all(|item| valid_block(item, plan, false)))
        })
}

fn valid_block(value: &Value, plan: bool, system: bool) -> bool {
    let Some(block) = value.as_object() else {
        return false;
    };
    let Some(kind) = block.get("type").and_then(Value::as_str) else {
        return false;
    };
    if kind == "text" {
        return fields(block, &["type", "text", "cache_control"])
            && block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| system || nonempty(text))
            && valid_marker(block);
    }
    if kind == "thinking" {
        return plan
            && !system
            && fields(block, &["type", "thinking", "signature"])
            && block.get("thinking").is_some_and(Value::is_string)
            && block
                .get("signature")
                .and_then(Value::as_str)
                .is_some_and(nonempty);
    }
    if kind == "tool_use" {
        return !system
            && fields(block, &["type", "id", "name", "input", "cache_control"])
            && block
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(nonempty)
            && block
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(nonempty)
            && block.get("input").is_some_and(Value::is_object)
            && valid_marker(block);
    }
    kind == "tool_result"
        && !system
        && fields(
            block,
            &[
                "type",
                "tool_use_id",
                "content",
                "is_error",
                "cache_control",
            ],
        )
        && block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .is_some_and(nonempty)
        && block.get("content").is_some_and(|item| {
            item.is_string()
                || item
                    .as_array()
                    .is_some_and(|items| items.iter().all(valid_result_text))
        })
        && block.get("is_error").is_none_or(Value::is_boolean)
        && valid_marker(block)
}

fn valid_result_text(value: &Value) -> bool {
    value.as_object().is_some_and(|item| {
        fields(item, &["type", "text"])
            && item.get("type").and_then(Value::as_str) == Some("text")
            && item.get("text").is_some_and(Value::is_string)
    })
}
fn fields(object: &JsonObject, allowed: &[&str]) -> bool {
    object.keys().all(|key| allowed.contains(&key.as_str()))
}
fn valid_marker(object: &JsonObject) -> bool {
    object
        .get("cache_control")
        .is_none_or(|item| cache_ttl(item).is_some())
}
pub(super) fn cache_ttl(value: &Value) -> Option<u32> {
    let object = value.as_object()?;
    if !fields(object, &["type", "ttl"])
        || object.get("type").and_then(Value::as_str) != Some("ephemeral")
    {
        return None;
    }
    match object.get("ttl").and_then(Value::as_str).unwrap_or("5m") {
        "5m" => Some(300),
        "1h" => Some(3600),
        _ => None,
    }
}
pub(super) fn nonempty(value: &str) -> bool {
    !value.is_empty() && !value.trim().is_empty()
}
fn integer(value: &Value) -> bool {
    value.as_i64().is_some() || value.as_u64().is_some()
}
fn number(value: &Value) -> bool {
    value.as_f64().is_some() && !value.is_boolean()
}
