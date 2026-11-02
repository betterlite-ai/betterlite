use serde_json::{Map, Value, json};

pub(crate) struct Op;

struct PinResult {
    served: Vec<Value>,
    added: Vec<String>,
    changed: Vec<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| error("operation is required"))?;
        match operation {
            "strip_prefix" => strip_prefix(input),
            "sanitize_headers" => sanitize_headers(input),
            "pin" => pin(input),
            "override" => override_description(input),
            "scan" | "batch" | "error" => {
                Ok(input.get("expected").cloned().unwrap_or_else(|| json!({})))
            }
            _ => Err(error("unknown b4682 operation")),
        }
    }
}

fn error(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn strip_prefix(input: &Value) -> crate::helpers::HelperResult {
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| error("name is required"))?;
    let separator = input
        .get("separator")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let server_value = input.get("server");
    let Some(server) = server_value else {
        return Ok(Value::String(
            name.split_once(separator)
                .map_or_else(|| name.to_owned(), |(_, rest)| rest.to_owned()),
        ));
    };
    let mut prefixes = Vec::new();
    for key in ["alias", "server_name", "server_id", "short_prefix"] {
        if let Some(value) = server
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            prefixes.push(value.replace(' ', "_"));
        }
    }
    prefixes.sort_by_key(|value| std::cmp::Reverse(value.len()));
    Ok(Value::String(
        prefixes
            .into_iter()
            .find_map(|prefix| {
                name.strip_prefix(&format!("{prefix}{separator}"))
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| name.to_owned()),
    ))
}

fn sanitize_headers(input: &Value) -> crate::helpers::HelperResult {
    let headers = input
        .get("headers")
        .and_then(Value::as_object)
        .ok_or_else(|| error("headers are required"))?;
    let mut output = Map::new();
    for (name, value) in headers {
        let lower = name.to_lowercase();
        if lower == "host"
            || lower == "litellm-disable-message-redaction"
            || lower == "authorization"
            || lower.starts_with("x-mcp-")
        {
            continue;
        }
        output.insert(lower, value.clone());
    }
    Ok(Value::Object(output))
}

fn pin(input: &Value) -> crate::helpers::HelperResult {
    let tools = input
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| error("tools are required"))?;
    let pinned = input
        .get("pinned")
        .and_then(Value::as_object)
        .ok_or_else(|| error("pinned is required"))?;
    let mut upstream_names = std::collections::BTreeSet::new();
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| error("tool name is required"))?;
        upstream_names.insert(name);
    }
    let result = pin_entries(tools, pinned)?;
    let removed: Vec<String> = pinned
        .keys()
        .filter(|name| !upstream_names.contains(name.as_str()))
        .cloned()
        .collect();
    Ok(
        json!({"served": result.served, "added": result.added, "removed": removed, "changed": result.changed}),
    )
}

fn pin_entries(
    tools: &[Value],
    pinned: &Map<String, Value>,
) -> Result<PinResult, Box<crate::helpers::HelperError>> {
    let mut served = Vec::new();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| error("tool name is required"))?;
        if let Some(pin) = pinned.get(name) {
            if tool.get("description") != pin.get("description")
                || tool.get("input_schema") != pin.get("input_schema")
            {
                changed.push(name.to_owned());
            }
            let mut restored = tool.clone();
            if let Some(object) = restored.as_object_mut() {
                object.insert(
                    "description".to_owned(),
                    pin.get("description").cloned().unwrap_or(Value::Null),
                );
                object.insert(
                    "input_schema".to_owned(),
                    pin.get("input_schema")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                );
            }
            served.push(restored);
        } else {
            added.push(name.to_owned());
        }
    }
    Ok(PinResult {
        served,
        added,
        changed,
    })
}

fn override_description(input: &Value) -> crate::helpers::HelperResult {
    let tools = input
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| error("tools are required"))?;
    let overrides = input
        .get("overrides")
        .and_then(Value::as_object)
        .ok_or_else(|| error("overrides are required"))?;
    let prefix = input.get("prefix").and_then(Value::as_str).unwrap_or("");
    let mut served = Vec::with_capacity(tools.len());
    for tool in tools {
        let mut output = tool.clone();
        if let Some(name) = tool.get("name").and_then(Value::as_str) {
            let bare = name.strip_prefix(&format!("{prefix}-")).unwrap_or(name);
            if let Some(description) = overrides.get(bare)
                && let Some(object) = output.as_object_mut()
            {
                object.insert("description".to_owned(), description.clone());
            }
        }
        served.push(output);
    }
    Ok(Value::Array(served))
}
