use betterlite_context::agentic_loop_plan::JsonMap;
use betterlite_models::decisions::Json;
use serde_json::from_str;
use thiserror::Error;

mod tests;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SafetyError {
    #[error("Agentic loop detected repeated tool-call fingerprint; aborting rerun")]
    RepeatedFingerprint,
    #[error("Exceeded max_agentic_loops={max_loops} for model={model}")]
    MaxLoopsExceeded { max_loops: u32, model: String },
}

pub(crate) fn check_loop_safety(
    tool_calls: &JsonMap,
    fingerprints: &[String],
    depth: u32,
    max_loops: u32,
    model: &str,
) -> Result<String, SafetyError> {
    let fingerprint = fingerprint_tools(tool_calls);
    if fingerprints.contains(&fingerprint) {
        return Err(SafetyError::RepeatedFingerprint);
    }
    if depth >= max_loops {
        return Err(SafetyError::MaxLoopsExceeded {
            max_loops,
            model: model.to_owned(),
        });
    }
    Ok(fingerprint)
}

// json.dumps(..., sort_keys=True, default=str) in compact form; the spaces json.dumps adds are not part of the contract
pub(crate) fn json_text(value: &Json) -> String {
    match value {
        Json::Null => String::from("null"),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => quote(text),
        Json::List(items) => {
            let parts: Vec<String> = items.iter().map(json_text).collect();
            format!("[{}]", parts.join(","))
        }
        Json::Object(entries) => {
            let mut parts: Vec<(&str, String)> = entries
                .iter()
                .map(|(key, value)| (key.as_str(), json_text(value)))
                .collect();
            parts.sort_unstable_by_key(|(key, _)| *key);
            let fields: Vec<String> = parts
                .iter()
                .map(|(key, value)| format!("{}:{value}", quote(key)))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
    }
}

fn fingerprint_tools(tool_calls: &JsonMap) -> String {
    let fields = tool_calls
        .iter()
        .map(|(key, value)| {
            // the text fallback is json.dumps(default=str) for a value JSON cannot hold, which RawValue never is
            (
                key.clone(),
                from_str::<Json>(value.get())
                    .unwrap_or_else(|_parse_error| Json::Text(value.get().to_owned())),
            )
        })
        .collect();
    json_text(&Json::Object(fields))
}

fn quote(text: &str) -> String {
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{08}' => quoted.push_str("\\b"),
            '\u{0c}' => quoted.push_str("\\f"),
            control if control < ' ' => {
                let escape = format!("\\u{:04x}", u32::from(control));
                quoted.push_str(&escape);
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}
