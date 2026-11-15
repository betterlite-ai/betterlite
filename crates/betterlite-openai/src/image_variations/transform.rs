use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::image_generation::reply::ImageUsage;

// the reply's usage survives as the SDK sent it, and the SDK adds a null output_tokens_details
#[must_use]
pub fn usage_identity(usage: ImageUsage, response: &Json) -> ImageUsage {
    let Some(wire) = usage_object(response) else {
        return ImageUsage {
            prompt_tokens: None,
            completion_tokens: None,
            extra: IndexMap::new(),
            ..usage
        };
    };
    let mut extra = wire.clone();
    for name in [
        "input_tokens",
        "input_tokens_details",
        "output_tokens",
        "total_tokens",
        "prompt_tokens",
        "completion_tokens",
    ] {
        extra.shift_remove(name);
    }
    extra
        .entry(String::from("output_tokens_details"))
        .or_insert(Json::Null);
    ImageUsage {
        prompt_tokens: count(wire.get("prompt_tokens")),
        completion_tokens: count(wire.get("completion_tokens")),
        extra,
        ..usage
    }
}

fn usage_object(response: &Json) -> Option<&IndexMap<String, Json>> {
    let Json::Object(fields) = response else {
        return None;
    };
    match fields.get("usage") {
        Some(Json::Object(usage)) => Some(usage),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    }
}

fn count(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}
