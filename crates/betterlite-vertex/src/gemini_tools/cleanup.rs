use betterlite_prompts::arguments::json::{Json, Object};

pub fn remove_unsupported_fields(tool: &mut Object) {
    if tool.get("additionalProperties") == Some(&Json::Bool(false)) {
        tool.shift_remove("additionalProperties");
    }
    tool.shift_remove("strict");
    for (key, value) in tool.iter_mut() {
        if let Json::Object(properties) = value
            && key == "properties"
        {
            properties.values_mut().for_each(strip_value);
        } else {
            strip_value(value);
        }
    }
}

fn strip_value(value: &mut Json) {
    match value {
        Json::Object(object) => remove_unsupported_fields(object),
        Json::List(items) => {
            for item in items {
                strip_value(item);
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}
