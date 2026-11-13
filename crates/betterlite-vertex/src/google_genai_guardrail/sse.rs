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
    let joined: String = chunks.iter().map(|chunk| decode_lossy(chunk)).collect();
    let mut texts = Vec::new();
    for line in joined.split(|character| LINE_BOUNDARIES.contains(&character)) {
        let Some(payload) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        if let Ok(Json::Object(object)) = serde_json::from_str::<Json>(payload) {
            texts.extend(response_texts(&object).1);
        }
    }
    texts
}

// LiteLLM reads every object chunk first, then scans the raw frames joined in their own order
#[must_use]
pub fn accumulated_stream_text(frames: &[Json]) -> String {
    let mut objects = String::new();
    let mut raw: Vec<&[u8]> = Vec::new();
    for frame in frames {
        match frame {
            Json::Object(object) => objects.push_str(&response_texts(object).1.concat()),
            Json::Text(text) => raw.push(text.as_bytes()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
        }
    }
    objects + &decode_google_guardrail_sse(&raw).concat()
}
