use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::CountTokensError;
use super::{InputType, detect_input_type};

pub fn validate_count_tokens_request(
    request: &IndexMap<String, Json>,
) -> Result<(), CountTokensError> {
    let model_supplied = match request.get("model") {
        None | Some(Json::Null | Json::Bool(false)) => false,
        Some(Json::Bool(true)) => true,
        Some(Json::Number(number)) => number.as_f64().is_none_or(|value| value != 0.0_f64),
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(fields)) => !fields.is_empty(),
    };
    if !model_supplied {
        return Err(CountTokensError::ModelRequired);
    }
    if detect_input_type(request) == InputType::Converse {
        return validate_converse(request);
    }
    if request.len() <= 1 {
        return Err(CountTokensError::ContentRequired);
    }
    Ok(())
}

fn validate_converse(request: &IndexMap<String, Json>) -> Result<(), CountTokensError> {
    let messages: &[Json] = match request.get("messages") {
        Some(Json::List(messages)) => messages,
        None | Some(_) => &[],
    };
    if messages.is_empty() {
        return Err(CountTokensError::MessagesRequired);
    }
    for (index, message) in messages.iter().enumerate() {
        let Json::Object(fields) = message else {
            return Err(CountTokensError::MessageNotObject { index });
        };
        if !fields.contains_key("role") {
            return Err(CountTokensError::MessageFieldMissing {
                index,
                field: "role",
            });
        }
        if !fields.contains_key("content") {
            return Err(CountTokensError::MessageFieldMissing {
                index,
                field: "content",
            });
        }
    }
    Ok(())
}
