use std::cmp::Ordering;

use betterlite_models::decisions::Json;

use super::callback::GuardrailViolation;

pub(crate) const DEFAULT_BLOCK_MESSAGE: &str =
    "I'm sorry, that request was blocked by the content filter.";

const GUARDRAIL_PROMPT: &str =
    "Say exactly the following message to the user, word for word, do not add anything else: ";

// LiteLLM's error_msg: a non-empty realtime_violation_message wins, then the raised exception's detail
pub(crate) fn violation_message(violation: &GuardrailViolation, configured: Option<&str>) -> Json {
    if let Some(chosen) = configured.filter(|message| !message.is_empty()) {
        return Json::Text(chosen.to_owned());
    }
    detail_message(violation)
}

// LiteLLM's ladder: a dict detail uses its truthy error field, another detail is stringified, and only a missing detail defaults an empty str(exception)
fn detail_message(violation: &GuardrailViolation) -> Json {
    match &violation.detail {
        Json::Object(fields) => fields
            .get("error")
            .filter(|error| error_message_wins(error))
            .cloned()
            .unwrap_or_else(|| Json::Text(violation.message.clone())),
        Json::Null => exception_message(&violation.message),
        Json::Text(text) => Json::Text(text.clone()),
        detail @ Json::Bool(_) | detail @ Json::Number(_) | detail @ Json::List(_) => {
            Json::Text(serde_json::to_string(detail).unwrap_or_default())
        }
    }
}

fn exception_message(message: &str) -> Json {
    if message.is_empty() {
        Json::Text(DEFAULT_BLOCK_MESSAGE.to_owned())
    } else {
        Json::Text(message.to_owned())
    }
}

// The detail error wins unless it is null, false, zero or empty, which is how LiteLLM's `if detail_error:` behaves
fn error_message_wins(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => match (number.as_i64(), number.as_u64()) {
            (Some(integer), _) => integer != 0,
            (None, Some(integer)) => integer != 0,
            (None, None) => number
                .as_f64()
                .is_some_and(|float| float.partial_cmp(&0.0_f64) != Some(Ordering::Equal)),
        },
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

// The two control frames that bracket a block: the cancellation and the request for a fresh response
pub(crate) fn control_frames() -> (Json, Json) {
    let cancel = object(vec![("type", Json::Text(String::from("response.cancel")))]);
    let create = object(vec![("type", Json::Text(String::from("response.create")))]);
    (cancel, create)
}

pub(crate) fn spoken_message(message: &Json) -> Json {
    object(vec![
        ("type", Json::Text(String::from("conversation.item.create"))),
        (
            "item",
            object(vec![
                ("type", Json::Text(String::from("message"))),
                ("role", Json::Text(String::from("user"))),
                (
                    "content",
                    Json::List(vec![object(vec![
                        ("type", Json::Text(String::from("input_text"))),
                        (
                            "text",
                            Json::Text(format!("{GUARDRAIL_PROMPT}{}", prompt_text(message))),
                        ),
                    ])]),
                ),
            ]),
        ),
    ])
}

fn prompt_text(message: &Json) -> String {
    match message {
        Json::Text(text) => text.clone(),
        other @ Json::Null
        | other @ Json::Bool(_)
        | other @ Json::Number(_)
        | other @ Json::List(_)
        | other @ Json::Object(_) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn object(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}
