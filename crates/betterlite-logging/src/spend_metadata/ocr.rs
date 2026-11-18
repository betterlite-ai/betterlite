use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

// _extract_usage_for_ocr_call: the dict's usage_info wins when truthy, else the object attribute, and pages replace tokens.
#[must_use]
pub fn extract_usage_for_ocr_call(
    usage_info: Option<&Json>,
    attribute_usage_info: Option<&Json>,
) -> JsonObject {
    let resolved = match usage_info {
        Some(Json::Bool(true)) => usage_info,
        Some(Json::Number(number)) if number.as_f64().is_none_or(|value| value != 0.0_f64) => {
            usage_info
        }
        Some(Json::Text(text)) if !text.is_empty() => usage_info,
        Some(Json::List(items)) if !items.is_empty() => usage_info,
        Some(Json::Object(entries)) if !entries.is_empty() => usage_info,
        Some(
            Json::Null
            | Json::Bool(false)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => attribute_usage_info.or(usage_info),
    };
    match resolved {
        None | Some(Json::Null) => JsonObject::new(),
        Some(Json::Object(entries)) => {
            let mut usage = zeroed_token_usage();
            for (key, value) in entries {
                usage.insert(key.clone(), value.clone());
            }
            usage
                .entry("pages_processed".to_owned())
                .or_insert_with(|| Json::Number(Number::from(0_u64)));
            usage
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            let mut usage = zeroed_token_usage();
            usage.insert(
                "pages_processed".to_owned(),
                Json::Number(Number::from(0_u64)),
            );
            usage
        }
    }
}

fn zeroed_token_usage() -> JsonObject {
    let mut usage = JsonObject::new();
    for counter in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        usage.insert(counter.to_owned(), Json::Number(Number::from(0_u64)));
    }
    usage
}

#[path = "ocr/tests.rs"]
mod tests;
