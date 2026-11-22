use super::CUSTOM_ID_LOG_LIMIT;
use super::record_json::dumps::number_str;
use super::record_json::text::RecordText;
use super::record_json::{RecordJson, RecordObject};

// Callers send numbers too, and a null would empty the one field they reconcile on; a lone surrogate would fail the response
#[must_use]
pub fn custom_id_of(payload: &RecordObject) -> Option<String> {
    match payload.get(&RecordText::from("custom_id")) {
        Some(RecordJson::Text(text)) => Some(text.replaced('?')),
        Some(RecordJson::Number(literal)) => Some(number_str(literal)),
        Some(
            RecordJson::Null | RecordJson::Bool(_) | RecordJson::List(_) | RecordJson::Object(_),
        )
        | None => None,
    }
}

// custom_id is caller-supplied and reaches a log line, so control characters become spaces and the id is capped
#[must_use]
pub fn describe(custom_id: Option<&str>) -> String {
    let Some(shown) = custom_id.filter(|text| !text.is_empty()) else {
        return String::new();
    };
    let safe: String = shown
        .chars()
        .take(CUSTOM_ID_LOG_LIMIT)
        .map(|character| {
            if character.is_ascii_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    format!(" (custom_id {safe})")
}
