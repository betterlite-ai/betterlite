use betterlite_prompts::arguments::json::{Json, Object};

// get_type_hints(Schema) in litellm/types/llms/vertex_ai.py: the only keys Vertex's Schema accepts
const VALID_SCHEMA_FIELDS: [&str; 22] = [
    "anyOf",
    "default",
    "description",
    "enum",
    "example",
    "format",
    "items",
    "maxItems",
    "maxLength",
    "maxProperties",
    "maximum",
    "minItems",
    "minLength",
    "minProperties",
    "minimum",
    "nullable",
    "pattern",
    "properties",
    "propertyOrdering",
    "required",
    "title",
    "type",
];

pub(crate) fn filter_schema_fields(schema: &Object) -> Object {
    let filtered = filter_anyof_fields(schema);
    let mut result = Object::new();
    for (key, value) in &filtered {
        if !VALID_SCHEMA_FIELDS.contains(&key.as_str()) {
            continue;
        }
        let kept = match (key.as_str(), value) {
            ("format", Json::Text(format)) if format == "enum" || format == "date-time" => {
                value.clone()
            }
            ("format", _) => continue,
            ("properties", Json::Object(properties)) => {
                let mut kept_properties = Object::new();
                for (name, property) in properties {
                    kept_properties.insert(name.clone(), filtered_value(property));
                }
                Json::Object(kept_properties)
            }
            ("items", Json::Object(items)) => Json::Object(filter_schema_fields(items)),
            ("anyOf", Json::List(branches)) => {
                Json::List(branches.iter().map(filtered_value).collect())
            }
            (_, other) => other.clone(),
        };
        result.insert(key.clone(), kept);
    }
    result
}

fn filtered_value(value: &Json) -> Json {
    match value {
        Json::Object(object) => Json::Object(filter_schema_fields(object)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            value.clone()
        }
    }
}

// Vertex rejects a schema that keeps anything beside anyOf, so a node with anyOf keeps only the branches and their metadata
fn filter_anyof_fields(schema: &Object) -> Object {
    let Some(Json::List(branches)) = schema.get("anyOf") else {
        return schema.clone();
    };
    let title = schema
        .get("title")
        .filter(|value| !matches!(value, Json::Null));
    let description = schema
        .get("description")
        .filter(|value| !matches!(value, Json::Null));
    let mut copied = branches.clone();
    let all_objects = copied
        .iter()
        .all(|branch| matches!(branch, Json::Object(_)));
    if all_objects && (title.is_some() || description.is_some()) {
        copy_metadata(&mut copied, title, description);
    }
    let mut result = Object::new();
    result.insert("anyOf".to_owned(), Json::List(copied));
    result
}

fn copy_metadata(branches: &mut [Json], title: Option<&Json>, description: Option<&Json>) {
    for branch in branches {
        let Json::Object(object) = branch else {
            continue;
        };
        if let Some(title_text) = title {
            object.insert("title".to_owned(), title_text.clone());
        }
        if let Some(description_text) = description {
            object.insert("description".to_owned(), description_text.clone());
        }
    }
}
