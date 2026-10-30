use serde::Deserialize;
use serde_json::{Map, Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    stream: Option<String>,
    requests: Option<String>,
}

fn object(value: Option<&Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(entries)) => entries.clone(),
        _ => Map::new(),
    }
}

fn text(entries: &Map<String, Value>, key: &str) -> String {
    match entries.get(key).and_then(Value::as_str) {
        Some(value) => value.to_owned(),
        None => String::new(),
    }
}

fn normalized_name(native: &str) -> &str {
    match native {
        "multiedit" | "patch" | "apply_patch" | "edit" => "edit",
        "list" => "ls",
        "webfetch" | "websearch" => "web_search",
        _ => native,
    }
}

fn builtin(native: &str) -> bool {
    match native {
        "read" | "write" | "edit" | "multiedit" | "patch" | "apply_patch" | "bash" | "glob"
        | "grep" | "list" | "webfetch" | "websearch" | "task" | "todowrite" | "todoread"
        | "skill" | "invalid" | "question" | "lsp" | "codesearch" | "plan_enter" | "plan_exit" => {
            true
        }
        _ => false,
    }
}

fn tool_events(part: &Map<String, Value>) -> Vec<Value> {
    let native = text(part, "tool");
    let call_id = {
        let primary = text(part, "callID");
        if primary.is_empty() {
            text(part, "id")
        } else {
            primary
        }
    };
    let state = object(part.get("state"));
    let input = match state.get("input") {
        Some(Value::Object(entries)) => Value::Object(entries.clone()),
        Some(value) => json!({"input": value}),
        None => json!({}),
    };
    let call = json!({"type":"tool_call", "id":call_id, "name":normalized_name(&native),
        "native_name":native, "input":input, "builtin":builtin(&native)});
    let result = if state.get("status").and_then(Value::as_str) == Some("error") {
        let error_text = text(&state, "error");
        let status_text = if error_text.is_empty() {
            text(&state, "output")
        } else {
            error_text
        };
        let result_text = if status_text.is_empty() {
            "tool failed".to_owned()
        } else {
            status_text
        };
        json!({"type":"tool_result", "id":call_id, "output":result_text, "is_error":true})
    } else {
        let output_value = state
            .get("output")
            .map_or_else(|| Value::Null, Clone::clone);
        let output_text = match output_value {
            Value::String(value) => value,
            Value::Null => "null".to_owned(),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            Value::Array(value) => Value::Array(value).to_string(),
            Value::Object(value) => Value::Object(value).to_string(),
        };
        json!({"type":"tool_result", "id":call_id, "output":output_text, "is_error":native == "invalid"})
    };
    vec![call, result]
}

fn stream_event(
    line_object: &Map<String, Value>,
    step_texts: &mut Vec<String>,
    final_text: &mut String,
    error: &mut Option<String>,
    events: &mut Vec<Value>,
) {
    let part = object(line_object.get("part"));
    match line_object.get("type").and_then(Value::as_str) {
        Some("step_start") => step_texts.clear(),
        Some("text") => {
            let value = text(&part, "text");
            if !value.is_empty() {
                step_texts.push(value.clone());
                *final_text = step_texts.join("\n\n");
                events.push(json!({"type":"text", "delta":value}));
            }
        }
        Some("reasoning") => {
            let value = text(&part, "text");
            if !value.is_empty() {
                events.push(json!({"type":"reasoning", "delta":value}));
            }
        }
        Some("tool_use") => events.extend(tool_events(&part)),
        Some("error") => {
            let details = object(line_object.get("error"));
            let data = object(details.get("data"));
            let value = if !text(&data, "message").is_empty() {
                text(&data, "message")
            } else if !text(&details, "name").is_empty() {
                text(&details, "name")
            } else {
                "opencode reported an error".to_owned()
            };
            *error = Some(
                error
                    .take()
                    .map_or_else(|| value.clone(), |existing| format!("{existing}\n{value}")),
            );
        }
        _ => {}
    }
}

fn stream(input: &str) -> Result<Value, serde_json::Error> {
    let mut session_id = None;
    let mut final_text = String::new();
    let mut error = None;
    let mut step_texts = Vec::new();
    let mut events = Vec::new();
    for raw_line in input.lines() {
        let line: Value = serde_json::from_str(raw_line)?;
        let line_object = object(Some(&line));
        if session_id.is_none() {
            session_id = line_object
                .get("sessionID")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        stream_event(
            &line_object,
            &mut step_texts,
            &mut final_text,
            &mut error,
            &mut events,
        );
    }
    Ok(json!({"session_id":session_id, "final_text":final_text, "error":error, "events":events}))
}

fn requests(input: &str) -> Result<Value, serde_json::Error> {
    let mut result = Vec::new();
    for raw_line in input.lines() {
        let request: Value = serde_json::from_str(raw_line)?;
        let request_object = object(Some(&request));
        let headers = object(request_object.get("headers"));
        result.push(json!({"auth_prefix":request_object.get("auth_prefix"),
            "content_type":headers.get("Content-Type"), "tools":request_object.get("tools").cloned().unwrap_or_else(|| json!([])),
            "n_messages":request_object.get("n_messages"), "roles":request_object.get("roles").cloned().unwrap_or_else(|| json!([]))}));
    }
    Ok(json!({"requests":result}))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match (&input.stream, &input.requests) {
            (Some(stream_input), None) => stream(stream_input),
            (None, Some(request_input)) => requests(request_input),
            _ => {
                return Err(Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: "b4836 input must contain exactly one stream or requests string"
                        .to_owned(),
                }));
            }
        };
        result
            .map(|value| json!({"result":value}))
            .map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })
    }
}
