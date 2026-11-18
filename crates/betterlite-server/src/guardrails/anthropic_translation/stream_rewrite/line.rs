use std::str;

use serde_json::Value;

use super::events::rewritten_event;
use super::json_dumps::to_python_json;
use super::rewriter::EventRewriter;

// LiteLLM's _rewrite_sse_events: framing stays byte for byte, and bytes that are not UTF-8 come back unchanged.
#[must_use]
pub fn rewrite_sse_events(bytes: &[u8], rewriter: &mut EventRewriter) -> Vec<u8> {
    let Ok(source) = str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    source
        .split("\n\n")
        .map(|block| {
            block
                .split('\n')
                .map(|line| rewrite_sse_line(line, rewriter))
                .collect::<Vec<String>>()
                .join("\n")
        })
        .collect::<Vec<String>>()
        .join("\n\n")
        .into_bytes()
}

// LiteLLM's _rewrite_sse_line: only an untrimmed `data:` line whose payload is a JSON object can be rewritten.
#[must_use]
pub fn rewrite_sse_line(line: &str, rewriter: &mut EventRewriter) -> String {
    let Some(payload) = line.strip_prefix("data:") else {
        return line.to_owned();
    };
    let Ok(Value::Object(event)) = serde_json::from_str::<Value>(payload.trim()) else {
        return line.to_owned();
    };
    let Some(rewritten) = rewritten_event(&event, rewriter) else {
        return line.to_owned();
    };
    to_python_json(&rewritten).map_or_else(|_error| line.to_owned(), |text| format!("data: {text}"))
}
