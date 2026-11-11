use betterlite_models::controls::json_value::JsonValue;

use super::JsonObject;

fn edit_every_object<F>(schema: &mut JsonValue, edit: &F)
where
    F: Fn(&mut JsonObject),
{
    match schema {
        JsonValue::Object(fields) => {
            edit(fields);
            for value in fields.values_mut() {
                edit_every_object(value, edit);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                edit_every_object(item, edit);
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) => {}
    }
}

// Gemini and vertex reject `additionalProperties: false`; any other value of the key stays.
pub fn remove_additional_properties(schema: &mut JsonValue) {
    edit_every_object(schema, &|fields| {
        if fields.get("additionalProperties") == Some(&JsonValue::Bool(false)) {
            fields.shift_remove("additionalProperties");
        }
    });
}

pub fn remove_strict_from_schema(schema: &mut JsonValue) {
    edit_every_object(schema, &|fields| {
        fields.shift_remove("strict");
    });
}
