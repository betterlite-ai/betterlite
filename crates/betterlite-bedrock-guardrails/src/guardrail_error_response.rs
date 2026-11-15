use std::str::from_utf8;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const DEFAULT_MESSAGE: &str = "Bedrock guardrail request failed";

#[must_use]
pub fn parse_error_response(status_code: u16, body: &[u8]) -> (u16, String) {
    // an error body that is not UTF-8 takes the default message rather than a replacement-decoded one
    let Ok(text) = from_utf8(body) else {
        return (status_code, DEFAULT_MESSAGE.to_owned());
    };
    let fields = match serde_json::from_str::<Json>(text) {
        Ok(Json::Object(fields)) => fields,
        Ok(_) => return (status_code, DEFAULT_MESSAGE.to_owned()),
        Err(_) => {
            let trimmed = text.trim();
            return if trimmed.is_empty() {
                (status_code, DEFAULT_MESSAGE.to_owned())
            } else {
                (status_code, trimmed.to_owned())
            };
        }
    };
    (status_code, message_from(&fields))
}

fn message_from(fields: &IndexMap<String, Json>) -> String {
    if let Some(Json::Text(message)) = fields.get("message") {
        return message.clone();
    }
    match fields.get("error") {
        Some(Json::Object(error)) => match error.get("message") {
            Some(Json::Text(message)) => message.clone(),
            _ => DEFAULT_MESSAGE.to_owned(),
        },
        Some(Json::Text(error)) => error.clone(),
        _ => DEFAULT_MESSAGE.to_owned(),
    }
}
