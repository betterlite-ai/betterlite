use betterlite_models::decisions::Json as WireJson;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::{Json, Object};

// The prompts crate owns its own Json, and its public conversion covers the prompts-to-wire direction only.
pub(super) fn prompt_json(value: &WireJson) -> Json {
    match value {
        WireJson::Null => Json::Null,
        WireJson::Bool(flag) => Json::Bool(*flag),
        WireJson::Number(number) => Json::Number(number.clone()),
        WireJson::Text(text) => Json::Text(text.clone()),
        WireJson::List(items) => Json::List(items.iter().map(prompt_json).collect()),
        WireJson::Object(fields) => Json::Object(prompt_object(fields)),
    }
}

pub(super) fn prompt_object(fields: &JsonObject) -> Object {
    let mut object = Object::new();
    for (key, value) in fields {
        object.insert(key.clone(), prompt_json(value));
    }
    object
}

pub(super) fn wire_object(fields: &Object) -> JsonObject {
    let mut object = JsonObject::new();
    for (key, value) in fields {
        object.insert(key.clone(), WireJson::from(value));
    }
    object
}
