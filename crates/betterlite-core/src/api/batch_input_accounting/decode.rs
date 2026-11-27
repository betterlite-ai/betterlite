use std::str::from_utf8;

use betterlite_models::decisions::Json;

use super::DecodedBatchInputLine;

// Python's bytes.strip(): space, tab, newline, carriage return, vertical tab and form feed
const PYTHON_SPACE: &[u8] = b" \t\n\r\x0b\x0c";

pub(super) fn stripped_line(raw: &[u8]) -> &[u8] {
    let Some(first) = raw.iter().position(|byte| !PYTHON_SPACE.contains(byte)) else {
        return &[];
    };
    let last = raw
        .iter()
        .rposition(|byte| !PYTHON_SPACE.contains(byte))
        .unwrap_or(first);
    raw.get(first..=last).unwrap_or_default()
}

pub(super) fn decode_line(line: &[u8]) -> DecodedBatchInputLine<'_> {
    let parsed = from_utf8(line)
        .ok()
        .map(|text| text.strip_prefix('\u{feff}').unwrap_or(text))
        .and_then(|document| serde_json::from_str::<Json>(document).ok());
    match parsed {
        Some(Json::Object(entry)) => DecodedBatchInputLine::Entry { entry, raw: line },
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => DecodedBatchInputLine::Fallback { raw: line },
    }
}
