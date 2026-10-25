use serde_json::{Map, Value};

const TEXT_TYPES: [&str; 5] = [
    "text",
    "input_text",
    "output_text",
    "summary_text",
    "reasoning_text",
];

fn text_type(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| TEXT_TYPES.contains(&value))
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(name))
}

fn collect_strings(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::String(value) if !value.is_empty() => output.push(value.clone()),
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_strings(value, output)),
        Value::Object(object) => object
            .values()
            .for_each(|value| collect_strings(value, output)),
        _ => {}
    }
}

pub(super) fn normalize_message_content(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Array(values) => {
            let mut parts = Vec::new();
            for part in values {
                let Some(object) = part.as_object() else {
                    continue;
                };
                if text_type(object.get("type")) {
                    if let Some(text) = object
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                    {
                        parts.push(text.to_owned());
                    }
                    continue;
                }
                for name in ["content", "arguments", "output"] {
                    if let Some(nested) = object.get(name) {
                        let text = normalize_message_content(Some(nested));
                        if !text.is_empty() {
                            parts.push(text);
                        }
                    }
                }
            }
            parts.join(" ")
        }
        _ => value.to_string(),
    }
}

pub(super) fn extract_message_tool_argument_parts(value: &Value) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(calls) = field(value, "tool_calls").and_then(Value::as_array) {
        for call in calls {
            if let Some(arguments) =
                field(call, "function").and_then(|function| field(function, "arguments"))
            {
                if let Some(text) = arguments.as_str().filter(|text| !text.is_empty()) {
                    parts.push(text.to_owned());
                }
            }
        }
    }
    if let Some(arguments) = field(value, "function_call").and_then(|call| field(call, "arguments"))
    {
        if let Some(text) = arguments.as_str().filter(|text| !text.is_empty()) {
            parts.push(text.to_owned());
        }
    }
    parts
}

pub(super) fn extract_message_reasoning_parts(value: &Value) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(text) = field(value, "reasoning_content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        parts.push(text.to_owned());
    }
    for block in field(value, "thinking_blocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for name in ["thinking", "reasoning", "text"] {
            if let Some(text) = field(block, name)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                parts.push(text.to_owned());
            }
        }
    }
    for item in field(value, "reasoning_items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for block in field(item, "summary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(text) = field(block, "text")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                parts.push(text.to_owned());
            }
        }
        for name in ["text", "reasoning", "reasoning_content"] {
            if let Some(text) = field(item, name)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                parts.push(text.to_owned());
            }
        }
    }
    parts
}

fn message(role: &str, content: String) -> Value {
    Value::Object(Map::from_iter([
        ("role".to_owned(), Value::String(role.to_owned())),
        ("content".to_owned(), Value::String(content)),
    ]))
}

pub(super) fn flatten_responses_input(value: Option<&Value>) -> Vec<Value> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(text) = value.as_str() {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![message("user", text.to_owned())]
        };
    }
    if let Some(values) = value.as_array() {
        if values.iter().any(|item| field(item, "role").is_some()) {
            return values
                .iter()
                .filter_map(|item| {
                    let role = field(item, "role")
                        .and_then(Value::as_str)
                        .unwrap_or("user");
                    let text = normalize_message_content(Some(item));
                    (!text.is_empty()).then(|| message(role, text))
                })
                .collect();
        }
        let text = normalize_message_content(Some(value));
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![message("user", text)]
        };
    }
    if value.is_null() {
        return Vec::new();
    }
    let text = value.to_string();
    if text.is_empty() {
        Vec::new()
    } else {
        vec![message("user", text)]
    }
}

pub(super) fn extract_inspect_messages_from_request(value: &Value) -> Vec<Value> {
    let mut messages = Vec::new();
    let instruction = normalize_message_content(field(value, "instructions"));
    if !instruction.is_empty() {
        messages.push(message("system", instruction));
    }
    if let Some(items) = field(value, "messages").and_then(Value::as_array) {
        for item in items {
            let Some(role) = field(item, "role").and_then(Value::as_str) else {
                continue;
            };
            let mut parts = Vec::new();
            let text = normalize_message_content(field(item, "content"));
            if !text.is_empty() {
                parts.push(text);
            }
            parts.extend(extract_message_tool_argument_parts(item));
            if !parts.is_empty() {
                messages.push(message(role, parts.join(" ")));
            }
        }
    }
    if field(value, "input").is_some() {
        messages.extend(flatten_responses_input(field(value, "input")));
    }
    if messages.is_empty() {
        let text = normalize_message_content(field(value, "prompt"));
        if !text.is_empty() {
            messages.push(message("user", text));
        }
    }
    let mut tool_text = Vec::new();
    for name in ["tools", "functions"] {
        if let Some(value) = field(value, name) {
            collect_strings(value, &mut tool_text);
        }
    }
    if !tool_text.is_empty() {
        messages.push(message("system", tool_text.join(" ")));
    }
    messages
}

pub(super) fn extract_response_messages(value: &Value) -> Vec<Value> {
    let mut parts = Vec::new();
    if let Some(choices) = field(value, "choices").and_then(Value::as_array) {
        for choice in choices {
            if let Some(message_value) = field(choice, "message") {
                let text = normalize_message_content(field(message_value, "content"));
                if !text.is_empty() {
                    parts.push(text);
                }
                parts.extend(extract_message_tool_argument_parts(message_value));
                parts.extend(extract_message_reasoning_parts(message_value));
            } else if let Some(text) = field(choice, "text")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                parts.push(text.to_owned());
            }
        }
    } else if let Some(output) = field(value, "output").and_then(Value::as_array) {
        for item in output {
            if let Some(content) = field(item, "content").and_then(Value::as_array) {
                for part in content {
                    for name in ["text", "reasoning", "thinking"] {
                        if let Some(text) = field(part, name)
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                        {
                            parts.push(text.to_owned());
                        }
                    }
                }
            }
            for name in ["arguments", "text", "reasoning", "thinking"] {
                if let Some(text) = field(item, name)
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    parts.push(text.to_owned());
                }
            }
        }
    }
    if parts.is_empty() {
        Vec::new()
    } else {
        vec![message("assistant", parts.join(" "))]
    }
}

pub(super) fn sanitized_messages_to_responses_input(value: &Value) -> Option<Value> {
    let mut output = Vec::new();
    for item in value.as_array()? {
        let role = field(item, "role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let content = field(item, "content")?;
        if let Some(text) = content.as_str() {
            let kind = if role == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            output
                .push(serde_json::json!({"role": role, "content": [{"type": kind, "text": text}]}));
        } else if content.is_array() {
            output.push(serde_json::json!({"role": role, "content": content}));
        }
    }
    (!output.is_empty()).then_some(Value::Array(output))
}

pub(super) fn rewrite_responses_input_text(input: &Value, sanitized: &str) -> Option<Value> {
    if input.is_string() {
        return Some(Value::String(sanitized.to_owned()));
    }
    let values = input.as_array()?;
    let has_messages = values.iter().any(|item| field(item, "role").is_some());
    let mut rewritten = values.clone();
    if has_messages {
        for index in (0..rewritten.len()).rev() {
            let item = rewritten.get(index)?.as_object()?.clone();
            if item.get("role").and_then(Value::as_str) != Some("user") {
                continue;
            }
            let content = item.get("content")?;
            if content.is_string() {
                let mut replacement = item;
                replacement.insert("content".to_owned(), Value::String(sanitized.to_owned()));
                rewritten[index] = Value::Object(replacement);
                return Some(Value::Array(rewritten));
            }
            if let Some(parts) = content.as_array() {
                let mut parts = parts.clone();
                for part_index in 0..parts.len() {
                    if text_type(field(parts.get(part_index)?, "type")) {
                        let mut part = parts.get(part_index)?.as_object()?.clone();
                        part.insert("text".to_owned(), Value::String(sanitized.to_owned()));
                        parts[part_index] = Value::Object(part);
                        let mut replacement = item;
                        replacement.insert("content".to_owned(), Value::Array(parts));
                        rewritten[index] = Value::Object(replacement);
                        return Some(Value::Array(rewritten));
                    }
                }
            }
        }
        return None;
    }
    for index in 0..rewritten.len() {
        if text_type(field(rewritten.get(index)?, "type")) {
            let mut part = rewritten.get(index)?.as_object()?.clone();
            part.insert("text".to_owned(), Value::String(sanitized.to_owned()));
            rewritten[index] = Value::Object(part);
            return Some(Value::Array(rewritten));
        }
    }
    None
}

pub(super) fn run(input: &Value) -> Result<Value, String> {
    let operation = input
        .get("function")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing function".to_owned())?;
    let data = input.get("value").unwrap_or(input);
    match operation {
        "normalize_message_content"
        | "normalize_message_content_string"
        | "normalize_message_content_list" => {
            Ok(Value::String(normalize_message_content(Some(data))))
        }
        "extract_message_tool_argument_parts" => Ok(Value::Array(
            extract_message_tool_argument_parts(data)
                .into_iter()
                .map(Value::String)
                .collect(),
        )),
        "extract_message_reasoning_parts" => Ok(Value::Array(
            extract_message_reasoning_parts(data)
                .into_iter()
                .map(Value::String)
                .collect(),
        )),
        "flatten_responses_input"
        | "flatten_responses_input_string_to_message"
        | "flatten_responses_input_message_array"
        | "flatten_responses_input_content_part_list"
        | "flatten_responses_input_non_list_non_string"
        | "flatten_responses_input_message_string_content" => {
            Ok(Value::Array(flatten_responses_input(Some(data))))
        }
        "extract_inspect_messages_from_request"
        | "extract_inspect_messages_from_request_chat"
        | "extract_inspect_messages_from_request_responses_input"
        | "extract_inspect_messages_from_request_prompt" => {
            Ok(Value::Array(extract_inspect_messages_from_request(data)))
        }
        "extract_response_messages"
        | "extract_response_messages_chat"
        | "extract_response_messages_text_completion"
        | "extract_response_messages_responses_api" => {
            Ok(Value::Array(extract_response_messages(data)))
        }
        "extract_tool_definition_text" => {
            let mut parts = Vec::new();
            for name in ["tools", "functions"] {
                if let Some(value) = field(data, name) {
                    collect_strings(value, &mut parts);
                }
            }
            Ok(Value::String(parts.join(" ")))
        }
        "sanitized_messages_to_responses_input" => {
            Ok(sanitized_messages_to_responses_input(data).unwrap_or(Value::Null))
        }
        "rewrite_responses_input_text" => Ok(rewrite_responses_input_text(
            data.get("input").unwrap_or(&Value::Null),
            data.get("sanitized_text")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .unwrap_or(Value::Null)),
        _ => Err(format!("unknown Cisco conversation function: {operation}")),
    }
}

#[used]
static RUN_ENTRY: fn(&Value) -> Result<Value, String> = run;

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn cisco_conversation_tests_core_shapes() {
        let input = json!({"function": "extract_response_messages_chat", "value": {"choices": [{"message": {"content": "done"}}]}});
        assert_eq!(
            super::run(&input).expect("helper output"),
            json!([{"role": "assistant", "content": "done"}])
        );
    }
}
