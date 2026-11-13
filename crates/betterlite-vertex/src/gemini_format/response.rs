use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::model_policy::capabilities::response_schema::supports_response_json_schema;
use crate::vertex_schema::build::{SchemaFormat, shape_parameters};
use crate::vertex_schema::error::VertexSchemaError;

#[derive(Debug, thiserror::Error)]
pub enum GeminiFormatError {
    #[error(transparent)]
    Schema(#[from] VertexSchemaError),
}

impl GeminiFormatError {
    // LiteLLM's schema shaping raises a bare ValueError, which the exception mapper turns into a 500
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Schema(_) => Some(500),
        }
    }
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
    let type_mime = match format.get("type").and_then(Json::as_str) {
        Some("json_object") => Some("application/json"),
        Some("text") => Some("text/plain"),
        _ => None,
    };
    if let Some(mime) = type_mime {
        optional_params.insert("response_mime_type".to_owned(), Json::Text(mime.to_owned()));
    }
    let candidate = schema_candidate(&format);
    if candidate.is_some() {
        optional_params.insert(
            "response_mime_type".to_owned(),
            Json::Text("application/json".to_owned()),
        );
    }
    match candidate {
        Some(Json::Object(schema)) if !schema.is_empty() => {
            apply_schema(schema, modern, optional_params, schema_settings)
        }
        _ => Ok(()),
    }
}

fn schema_candidate(format: &Object) -> Option<&Json> {
    format.get("response_schema").or_else(|| {
        if format.get("type").and_then(Json::as_str) != Some("json_schema") {
            return None;
        }
        format
            .get("json_schema")
            .and_then(Json::as_object)?
            .get("schema")
    })
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
