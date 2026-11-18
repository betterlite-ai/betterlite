use betterlite_prompts::arguments::json::Json;

use crate::request_policy::{Params, RequestValue};

// Metadata resolution and validation only read Text and Mapping, so a scalar's source text is never inspected
pub(super) fn request_value(value: &Json) -> RequestValue {
    match value {
        Json::Text(text) => RequestValue::Text(text.clone()),
        Json::Object(fields) => RequestValue::Mapping(
            fields
                .iter()
                .map(|(key, entry)| (key.clone(), request_value(entry)))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            RequestValue::Raw(String::new())
        }
    }
}

pub(super) fn request_params(fields: &betterlite_prompts::arguments::json::Object) -> Params {
    fields
        .iter()
        .map(|(key, value)| (key.clone(), request_value(value)))
        .collect()
}
