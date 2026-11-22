use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub(super) fn json_back(value: JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(flag),
        JsonValue::Number(number) => Json::Number(number),
        JsonValue::Text(text) => Json::Text(text),
        JsonValue::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(json_back(item));
            }
            Json::List(values)
        }
        JsonValue::Object(fields) => {
            let mut values = IndexMap::new();
            for (name, field) in fields {
                values.insert(name, json_back(field));
            }
            Json::Object(values)
        }
    }
}
