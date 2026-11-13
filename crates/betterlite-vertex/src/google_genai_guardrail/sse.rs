use betterlite_prompts::arguments::json::Json;

use super::records::response_texts;

// Python's str.splitlines() boundaries: SSE frames end at CR, LF or CRLF, and exotic separators keep the scan identical
const LINE_BOUNDARIES: [char; 10] = [
    '\n', '\r', '\u{000b}', '\u{000c}', '\u{001c}', '\u{001d}', '\u{001e}', '\u{0085}', '\u{2028}',
    '\u{2029}',
];

// LiteLLM decodes each chunk with errors="replace", so a multibyte character split across chunks becomes replacement characters
fn decode_lossy(bytes: &[u8]) -> String {
    let mut decoded = String::new();
    let mut remaining = bytes;
    loop {
        match std::str::from_utf8(remaining) {
            Ok(text) => {
                decoded.push_str(text);
                return decoded;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if let Some(prefix) = remaining
                    .get(..valid)
                    .and_then(|slice| std::str::from_utf8(slice).ok())
                {
                    decoded.push_str(prefix);
                }
                decoded.push('\u{fffd}');
                let invalid = error
                    .error_len()
                    .unwrap_or(remaining.len().saturating_sub(valid));
                let Some(rest) = remaining.get(valid.saturating_add(invalid)..) else {
                    return decoded;
                };
                remaining = rest;
            }
        }
    }
}

#[must_use]
pub fn decode_google_guardrail_sse(chunks: &[&[u8]]) -> Vec<String> {
    let mut joined = String::new();
    for chunk in chunks {
        joined.push_str(&decode_lossy(chunk));
    }
    let mut texts = Vec::new();
    for line in joined.split(|character| LINE_BOUNDARIES.contains(&character)) {
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(Json::Object(object)) = serde_json::from_str::<Json>(payload) else {
            continue;
        };
        texts.extend(response_texts(&object));
    }
    texts
}

// LiteLLM reads every object chunk first, then scans the raw frames joined in their own order
#[must_use]
pub fn accumulated_stream_text(frames: &[Json]) -> String {
    let mut object_texts = String::new();
    let mut raw_chunks: Vec<&[u8]> = Vec::new();
    for frame in frames {
        match frame {
            Json::Object(object) => {
                for text in response_texts(object) {
                    object_texts.push_str(&text);
                }
            }
            Json::Text(text) => raw_chunks.push(text.as_bytes()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
        }
    }
    let mut accumulated = object_texts;
    for text in decode_google_guardrail_sse(&raw_chunks) {
        accumulated.push_str(&text);
    }
    accumulated
}
