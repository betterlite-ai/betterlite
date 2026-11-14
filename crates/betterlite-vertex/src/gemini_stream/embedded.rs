use super::error::StreamError;
use betterlite_models::decisions::Json;
pub(super) fn check_embedded_error(value: &[u8]) -> Result<(), StreamError> {
    let Ok(Json::Object(fields)) = serde_json::from_slice::<Json>(value) else {
        return Ok(());
    };
    match fields.get("error") {
        Some(error) => Err(embedded(error.clone())),
        None => Ok(()),
    }
}
fn embedded(error: Json) -> StreamError {
    let Json::Object(fields) = &error else {
        return StreamError::Embedded {
            status: 500,
            message: format!(
                "Unexpected error format in mid-stream chunk: {}",
                text_field(Some(&error), "null")
            ),
        };
    };
    let status = fields.get("code").map_or(500, coercible_status);
    let label = text_field(fields.get("status"), "UNKNOWN");
    let message = text_field(fields.get("message"), "Unknown error");
    StreamError::Embedded {
        status,
        message: format!("{label} - {message}"),
    }
}
fn coercible_status(value: &Json) -> u16 {
    let code = match value {
        Json::Number(number) => number.as_i64(),
        Json::Text(text) => text.trim().parse::<i64>().ok(),
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    };
    match code {
        Some(code) => u16::try_from(code).unwrap_or(500),
        None => 500,
    }
}
fn text_field(value: Option<&Json>, fallback: &str) -> String {
    match value {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null) | None => fallback.to_owned(),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| fallback.to_owned()),
    }
}
