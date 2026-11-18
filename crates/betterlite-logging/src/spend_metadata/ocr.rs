use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

// _extract_usage_for_ocr_call: the dict's usage_info wins when truthy, else the object attribute, and pages replace tokens.
#[must_use]
pub fn extract_usage_for_ocr_call(
    usage_info: Option<&Json>,
    attribute_usage_info: Option<&Json>,
) -> JsonObject {
    let resolved = if python_truthy(usage_info) {
        usage_info
    } else {
        attribute_usage_info.or(usage_info)
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

// Python truthiness for the fallback test: only an empty text, list or mapping, zero or null is falsy.
fn python_truthy(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null) => false,
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => number.as_f64().is_none_or(|number| number != 0.0),
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(entries)) => !entries.is_empty(),
    }
}

#[path = "ocr/tests.rs"]
mod tests;
