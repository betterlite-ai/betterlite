use betterlite_models::controls::json_value::JsonValue;
use serde::Serialize;

// LiteLLM hands the seam plain Python objects, so a string that holds JSON reads back as the value
#[cfg_attr(test, mutants::skip)]
pub(super) fn normalized<T>(value: &T) -> JsonValue
where
    T: Serialize,
{
    let Ok(converted) = serde_json::to_value(value) else {
        return JsonValue::Array(Vec::new());
    };
    let Ok(mut json) = serde_json::from_value::<JsonValue>(converted) else {
        return JsonValue::Array(Vec::new());
    };
    normalize(&mut json);
    json
}

#[cfg_attr(test, mutants::skip)]
fn normalize(value: &mut JsonValue) {
    let replacement = match value {
        JsonValue::Text(text) => serde_json::from_str::<JsonValue>(text).ok(),
        JsonValue::Array(items) => {
            for item in items {
                normalize(item);
            }
            None
        }
        JsonValue::Object(fields) => {
            for item in fields.values_mut() {
                normalize(item);
            }
            None
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => None,
    };
    if let Some(parsed) = replacement {
        *value = parsed;
    }
}
