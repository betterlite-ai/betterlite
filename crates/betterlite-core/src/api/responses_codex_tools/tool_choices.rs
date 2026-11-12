use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const MODES: [&str; 3] = ["none", "auto", "required"];
const BARE_TYPES: [&str; 11] = [
    "file_search",
    "web_search_preview",
    "computer",
    "computer_use_preview",
    "computer_use",
    "web_search_preview_2025_03_11",
    "image_generation",
    "code_interpreter",
    "programmatic_tool_calling",
    "apply_patch",
    "shell",
];

fn object<const SIZE: usize>(entries: [(&str, Json); SIZE]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn nonempty_text(value: Option<&Json>) -> Option<&str> {
    if let Some(Json::Text(text)) = value
        && !text.is_empty()
    {
        return Some(text);
    }
    None
}

// LiteLLM passes a choice through when function.name is truthy, whatever its type
fn has_value(value: &Json) -> bool {
    match value {
        Json::Null | Json::Bool(false) => false,
        Json::Bool(true) => true,
        Json::Number(number) => number.as_f64() != Some(0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(entries) => !entries.is_empty(),
    }
}

fn named_function(name: Option<&str>) -> Json {
    let Some(function_name) = name else {
        return Json::Text("required".to_owned());
    };
    let function = object([("name", Json::Text(function_name.to_owned()))]);
    object([
        ("type", Json::Text("function".to_owned())),
        ("function", function),
    ])
}

#[must_use]
pub fn chat_tool_choice(tool_choice: &Json) -> Json {
    let Json::Object(fields) = tool_choice else {
        return tool_choice.clone();
    };
    if let Some(Json::Object(function)) = fields.get("function")
        && function.get("name").is_some_and(has_value)
    {
        return tool_choice.clone();
    }
    let Some(Json::Text(kind)) = fields.get("type") else {
        return tool_choice.clone();
    };
    match kind.as_str() {
        "auto" | "none" => Json::Text(kind.clone()),
        "required" | "tool" | "any" => Json::Text("required".to_owned()),
        "function" => named_function(nonempty_text(fields.get("name"))),
        "custom" => {
            let custom_name = match fields.get("custom") {
                Some(Json::Object(custom)) => nonempty_text(custom.get("name")),
                _ => None,
            };
            named_function(nonempty_text(fields.get("name")).or(custom_name))
        }
        _ => tool_choice.clone(),
    }
}

fn required_text<'fields>(
    fields: &'fields IndexMap<String, Json>,
    key: &str,
) -> Option<&'fields str> {
    if let Some(Json::Text(text)) = fields.get(key) {
        return Some(text);
    }
    None
}

fn allowed_choice(fields: &IndexMap<String, Json>) -> Option<Json> {
    let mode = required_text(fields, "mode").filter(|mode| matches!(*mode, "auto" | "required"))?;
    let Some(Json::List(tools)) = fields.get("tools") else {
        return None;
    };
    if !tools.iter().all(|tool| matches!(tool, Json::Object(_))) {
        return None;
    }
    Some(object([
        ("type", Json::Text("allowed_tools".to_owned())),
        ("mode", Json::Text(mode.to_owned())),
        ("tools", Json::List(tools.clone())),
    ]))
}

// the same choices the OpenAI SDK's ToolChoice union validates; pydantic drops every other key
fn valid_choice(tool_choice: &Json) -> Option<Json> {
    let fields = match tool_choice {
        Json::Object(fields) => fields,
        Json::Text(mode) => return MODES.contains(&mode.as_str()).then(|| tool_choice.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => return None,
    };
    let kind = required_text(fields, "type")?;
    match kind {
        "function" | "custom" => {
            let name = required_text(fields, "name")?;
            Some(object([
                ("type", Json::Text(kind.to_owned())),
                ("name", Json::Text(name.to_owned())),
            ]))
        }
        "mcp" => {
            let label = required_text(fields, "server_label")?;
            let mut choice = object([
                ("type", Json::Text("mcp".to_owned())),
                ("server_label", Json::Text(label.to_owned())),
            ]);
            match (&mut choice, fields.get("name")) {
                (_, None) => {}
                (Json::Object(entries), Some(name @ (Json::Null | Json::Text(_)))) => {
                    entries.insert("name".to_owned(), name.clone());
                }
                _ => return None,
            }
            Some(choice)
        }
        "allowed_tools" => allowed_choice(fields),
        bare if BARE_TYPES.contains(&bare) => Some(object([("type", Json::Text(kind.to_owned()))])),
        _ => None,
    }
}

// the Chat Completions form back to a Responses one: a named function or custom choice, else nothing
fn named_choice(
    original: &IndexMap<String, Json>,
    normalized: &IndexMap<String, Json>,
) -> Option<Json> {
    let Some(Json::Object(function)) = normalized.get("function") else {
        return None;
    };
    let name = required_text(function, "name")?;
    let kind = if required_text(original, "type") == Some("custom") {
        "custom"
    } else if required_text(normalized, "type") == Some("function") {
        "function"
    } else {
        return None;
    };
    Some(object([
        ("type", Json::Text(kind.to_owned())),
        ("name", Json::Text(name.to_owned())),
    ]))
}

#[must_use]
pub fn responses_tool_choice(tool_choice: &Json) -> Json {
    if let Some(valid) = valid_choice(tool_choice) {
        return valid;
    }
    let normalized = chat_tool_choice(tool_choice);
    if let Json::Text(mode) = &normalized
        && MODES.contains(&mode.as_str())
    {
        return normalized;
    }
    if let (Json::Object(original), Json::Object(fields)) = (tool_choice, &normalized)
        && let Some(named) = named_choice(original, fields)
    {
        return named;
    }
    Json::Text("auto".to_owned())
}
