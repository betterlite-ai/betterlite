use betterlite_models::decisions::Json;

use crate::count_tokens::error::CountTokensError;
use crate::count_tokens::wire::CountTokensBody;

pub fn validate_request(body: &CountTokensBody<'_>) -> Result<(), CountTokensError> {
    if body.model.is_empty() {
        return Err(CountTokensError::MissingModel);
    }
    if is_falsey(body.messages)
        && body.system.is_none_or(is_falsey)
        && body.tools.is_none_or(is_falsey)
    {
        return Err(CountTokensError::MissingMessages);
    }
    let Json::List(messages) = body.messages else {
        return Err(CountTokensError::MessagesNotList);
    };
    for (index, message) in messages.iter().enumerate() {
        let Json::Object(fields) = message else {
            return Err(CountTokensError::MessageNotDictionary { index });
        };
        if !fields.contains_key("role") {
            return Err(CountTokensError::MessageMissingRole { index });
        }
        if !fields.contains_key("content") {
            return Err(CountTokensError::MessageMissingContent { index });
        }
    }
    Ok(())
}

// LiteLLM's `not value` over a JSON value, kept because the validator's contract is written that way
pub(crate) fn is_falsey(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(found) => !*found,
        Json::Number(number) => number.as_f64() == Some(0.0_f64),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
    }
}
