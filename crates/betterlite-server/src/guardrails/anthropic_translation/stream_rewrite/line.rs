use std::str;

use super::events::rewritten_event;
use super::json_dumps::dumped_json;
use super::rewriter::EventRewriter;
use crate::guardrails::anthropic_translation::sse_events::parse_sse_data_line;

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
    if !line.starts_with("data:") {
        return line.to_owned();
    }
    let Some(event) = parse_sse_data_line(line) else {
        return line.to_owned();
    };
    let Some(rewritten) = rewritten_event(&event, rewriter) else {
        return line.to_owned();
    };
    dumped_json(&rewritten).map_or_else(|_error| line.to_owned(), |text| format!("data: {text}"))
}
