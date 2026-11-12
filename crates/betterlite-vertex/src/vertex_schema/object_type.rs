use betterlite_prompts::arguments::json::{Json, Object};

pub(crate) fn add_object_type(schema: &mut Object) {
    let has_alternatives = ["anyOf", "oneOf", "allOf"]
        .iter()
        .any(|key| schema.contains_key(*key));
    if !schema.contains_key("type") && !has_alternatives {
        schema.insert("type".to_owned(), Json::Text("object".to_owned()));
    }
    let properties_empty = match schema.get("properties") {
        Some(Json::Object(properties)) => Some(properties.is_empty()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    if let Some(empty) = properties_empty {
        if matches!(schema.get("required"), Some(Json::Null)) {
            schema.shift_remove("required");
        }
        if empty {
            schema.shift_remove("properties");
            schema.shift_remove("required");
        }
        schema.insert("type".to_owned(), Json::Text("object".to_owned()));
        if !empty && let Some(Json::Object(properties)) = schema.get_mut("properties") {
            add_type_to_properties(properties);
        }
    }
    if let Some(Json::Object(items)) = schema.get_mut("items") {
        add_object_type(items);
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(Json::List(branches)) = schema.get_mut(key) {
            add_type_to_branches(branches);
        }
    }
}

fn add_type_to_properties(properties: &mut Object) {
    for property in properties.values_mut() {
        if let Json::Object(child) = property {
            add_object_type(child);
        }
    }
}

fn add_type_to_branches(branches: &mut [Json]) {
    for branch in branches {
        if let Json::Object(child) = branch {
            add_object_type(child);
        }
    }
}
