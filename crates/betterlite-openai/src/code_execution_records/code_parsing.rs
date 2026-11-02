use serde_json::Value;

// Display-only fallback: a parse failure, a non-object payload, a missing code key or a non-string code value all collapse to "" rather than erroring, since _run_tool_call parses the same arguments strictly and is the path that actually rejects malformed input
#[must_use]
pub fn parse_code(arguments: &str) -> String {
    if arguments.is_empty() {
        return String::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(arguments) else {
        return String::new();
    };
    value
        .as_object()
        .and_then(|object| object.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

include!("code_parsing/tests/cases.rs");
