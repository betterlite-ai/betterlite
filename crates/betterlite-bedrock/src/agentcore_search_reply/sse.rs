use std::mem::take;

use betterlite_models::decisions::Json;

const DATA_PREFIX: &str = "data:";

const SSE_PREFIXES: [&str; 5] = ["event:", "data:", ":", "id:", "retry:"];

pub(crate) fn lstrip(text: &str) -> &str {
    text.trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    })
}

pub(crate) fn looks_like_sse(text: &str) -> bool {
    let stripped = lstrip(text);
    SSE_PREFIXES
        .iter()
        .any(|prefix| stripped.starts_with(*prefix))
}

pub(crate) fn iter_sse_events(text: &str) -> Vec<Json> {
    let mut events = Vec::new();
    for chunk in split_events(text) {
        let payload = data_payload(&chunk);
        if payload.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Json>(&payload) else {
            continue;
        };
        if matches!(parsed, Json::Object(_)) {
            events.push(parsed);
        }
    }
    events
}

fn split_events(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if content.chars().all(is_blank) {
            let keep = current
                .strip_suffix("\r\n")
                .or_else(|| current.strip_suffix('\n'))
                .map_or(current.len(), str::len);
            current.truncate(keep);
            chunks.push(take(&mut current));
        } else {
            current.push_str(line);
        }
    }
    chunks.push(current);
    chunks
}

fn data_payload(chunk: &str) -> String {
    chunk
        .split(is_line_boundary)
        .filter_map(|line| line.strip_prefix(DATA_PREFIX))
        .map(lstrip)
        .collect::<Vec<&str>>()
        .join("\n")
}

const fn is_blank(character: char) -> bool {
    matches!(character, ' ' | '\t')
}

const fn is_line_boundary(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1c}'
            ..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}
