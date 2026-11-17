use betterlite_prompts::arguments::json::{Json, Object};

use super::records::response_texts;

// LiteLLM decodes each chunk with errors="replace", so a multibyte character split across chunks becomes replacement characters
fn decode_lossy(bytes: &[u8]) -> String {
    let mut decoded = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        decoded.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            decoded.push(char::REPLACEMENT_CHARACTER);
        }
    }
    decoded
}

#[must_use]
pub fn decode_google_guardrail_sse(chunks: &[&[u8]]) -> Vec<String> {
    let joined: String = chunks.iter().copied().map(decode_lossy).collect();
    let mut texts = Vec::new();
    for line in joined.split(['\r', '\n']) {
        texts.extend(line_texts(line));
    }
    texts
}

// SSE lines end only at CR and LF, so U+001C to U+001F reach this trim and Python's strip removes them
fn payload_object(payload: &str) -> Option<Object> {
    let trimmed = payload.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    });
    match serde_json::from_str::<Json>(trimmed) {
        Ok(Json::Object(object)) => Some(object),
        Ok(_) | Err(_) => None,
    }
}

// LiteLLM's splitlines reads each frame as its own line, so a kept line is read as the data: fields it holds, cut only outside a JSON string
fn line_texts(line: &str) -> Vec<String> {
    let Some(payload) = line.strip_prefix("data:") else {
        return Vec::new();
    };
    if let Some(object) = payload_object(payload) {
        return response_texts(&object).1;
    }
    let mut starts = data_field_starts(line);
    let mut texts = Vec::new();
    starts.push(line.len());
    for (start, end) in starts.iter().zip(starts.iter().skip(1)) {
        let Some(field) = line.get(*start..*end) else {
            continue;
        };
        let Some(field_payload) = field.strip_prefix("data:") else {
            continue;
        };
        if let Some(object) = payload_object(field_payload) {
            texts.extend(response_texts(&object).1);
        }
    }
    texts
}

// every data: that starts a field of the line, which means it is not inside a JSON string
fn data_field_starts(line: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if character == '\\' {
                escaped = true;
                continue;
            }
            if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
            continue;
        }
        if line
            .get(index..)
            .is_some_and(|rest| rest.starts_with("data:"))
        {
            starts.push(index);
        }
    }
    starts
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
    objects.push_str(&decode_google_guardrail_sse(&raw).concat());
    objects
}
