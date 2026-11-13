use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::model_policy::capabilities::response_schema::supports_response_json_schema;
use crate::vertex_schema::build::{SchemaFormat, shape_parameters};

use super::error::GeminiFormatError;

enum SelectedSchema<'schema> {
    Absent,
    Empty,
    Schema(&'schema Object),
}

pub fn apply_response_format(
    response_format: &Object,
    model: &str,
    optional_params: &mut Object,
    schema_settings: &SchemaSettings,
) -> Result<(), GeminiFormatError> {
    let mut format = response_format.clone();
    visit_objects(&mut format, &mut |fields: &mut Object| {
        fields.shift_remove("strict");
    });
    let modern = supports_response_json_schema(model);
    if !modern {
        visit_objects(&mut format, &mut |fields: &mut Object| {
            if matches!(fields.get("additionalProperties"), Some(Json::Bool(false))) {
                fields.shift_remove("additionalProperties");
            }
        });
    }
    if let Some(mime) = type_mime_type(&format) {
        optional_params.insert("response_mime_type".to_owned(), Json::Text(mime.to_owned()));
    }
    match selected_schema(&format) {
        SelectedSchema::Absent => {}
        SelectedSchema::Empty => set_json_mime_type(optional_params),
        SelectedSchema::Schema(schema) => {
            set_json_mime_type(optional_params);
            apply_schema(schema, modern, optional_params, schema_settings)?;
        }
    }
    Ok(())
}

fn set_json_mime_type(optional_params: &mut Object) {
    optional_params.insert(
        "response_mime_type".to_owned(),
        Json::Text("application/json".to_owned()),
    );
}

fn apply_schema(
    schema: &Object,
    modern: bool,
    optional_params: &mut Object,
    schema_settings: &SchemaSettings,
) -> Result<(), GeminiFormatError> {
    if modern {
        optional_params.insert(
            "response_json_schema".to_owned(),
            Json::Object(schema.clone()),
        );
        return Ok(());
    }
    let shaped = shape_parameters(
        schema.clone(),
        SchemaFormat::ResponseSchema,
        true,
        schema_settings,
    )?;
    optional_params.insert("response_schema".to_owned(), Json::Object(shaped));
    Ok(())
}

fn selected_schema(format: &Object) -> SelectedSchema<'_> {
    let candidate = if format.contains_key("response_schema") {
        format.get("response_schema")
    } else if format.get("type").and_then(Json::as_str) == Some("json_schema") {
        format
            .get("json_schema")
            .and_then(Json::as_object)
            .and_then(|json_schema| json_schema.get("schema"))
    } else {
        return SelectedSchema::Absent;
    };
    match candidate {
        None => SelectedSchema::Absent,
        Some(Json::Object(schema)) if !schema.is_empty() => SelectedSchema::Schema(schema),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        ) => SelectedSchema::Empty,
    }
}

fn type_mime_type(format: &Object) -> Option<&'static str> {
    match format.get("type").and_then(Json::as_str) {
        Some("json_object") => Some("application/json"),
        Some("text") => Some("text/plain"),
        _ => None,
    }
}

fn visit_objects(fields: &mut Object, visit: &mut dyn FnMut(&mut Object)) {
    visit(fields);
    for child in fields.values_mut() {
        visit_value(child, visit);
    }
}

fn visit_value(value: &mut Json, visit: &mut dyn FnMut(&mut Object)) {
    match value {
        Json::Object(fields) => visit_objects(fields, visit),
        Json::List(items) => {
            for item in items {
                visit_value(item, visit);
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}
