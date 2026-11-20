use crate::stream_chunks::SSE_FRAME_DELIMITERS;

// LiteLLM's STREAM_SSE_KEEPALIVE_PING_CHUNK, one whole SSE event
pub const ANTHROPIC_PING_CHUNK: &str = "event: ping\ndata: {\"type\": \"ping\"}\n\n";
pub const ANTHROPIC_PING_BYTES: &[u8] = ANTHROPIC_PING_CHUNK.as_bytes();
pub const SSE_COMMENT_PING: &str = ": ping\n\n";
pub const SSE_STREAM_START_TAIL: &[u8] = b"\n\n";

// sse_keepalive.py's _SSE_DELIMITER_LOOKBACK: the longest delimiter, `\r\n\r\n`
const DELIMITER_LOOKBACK: usize = 4;

#[must_use]
pub fn advance_sse_tail(tail: &[u8], written: &[u8]) -> Vec<u8> {
    if written.is_empty() {
        return tail.to_vec();
    }
    let mut combined = Vec::with_capacity(tail.len().saturating_add(written.len()));
    combined.extend_from_slice(tail);
    combined.extend_from_slice(written);
    combined
        .get(combined.len().saturating_sub(DELIMITER_LOOKBACK)..)
        .unwrap_or_default()
        .to_vec()
}

#[must_use]
pub fn seal_open_sse_frame(tail: &[u8]) -> String {
    if SSE_FRAME_DELIMITERS
        .iter()
        .any(|delimiter| tail.ends_with(delimiter.as_bytes()))
    {
        return String::new();
    }
    let line_break = if tail.ends_with(b"\n") || tail.ends_with(b"\r") {
        ""
    } else {
        "\n"
    };
    format!("{line_break}{ANTHROPIC_PING_CHUNK}")
}
