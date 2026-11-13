use serde_json::{Map, Value};

use crate::guardrails::provider_forms::error::FormError;
use crate::guardrails::provider_forms::spec::{FormField, FormModel, FormShape};

// LiteLLM checks the depth when it enters a model, and the top-level model enters at depth 0
#[must_use]
pub fn project_model(model: &FormModel, max_depth: usize) -> Result<Map<String, Value>, FormError> {
    let mut fields = Map::new();
    for field in &model.fields {
        insert_field(&mut fields, field, max_depth, 0)?;
    }
    Ok(fields)
}

pub fn with_friendly_name(model: &FormModel, fields: &mut Map<String, Value>) {
    if let Some(name) = model.ui_friendly_name {
        fields.insert(
            "ui_friendly_name".to_owned(),
            Value::String(name.to_owned()),
        );
    }
}

fn insert_field(
    fields: &mut Map<String, Value>,
    field: &FormField,
    max_depth: usize,
    depth: usize,
) -> Result<(), FormError> {
    let mut entry = Map::new();
    entry.insert(
        "description".to_owned(),
        Value::String(field.description.unwrap_or(field.name).to_owned()),
    );
    entry.insert("required".to_owned(), Value::Bool(field.required));
    if let FormShape::Nested(nested) = &field.shape {
        if depth >= max_depth {
            return Err(FormError::TooDeep { limit: max_depth });
        }
        entry.insert("type".to_owned(), Value::String("nested".to_owned()));
        let mut nested_fields = Map::new();
        for nested_field in &nested.fields {
            insert_field(
                &mut nested_fields,
                nested_field,
                max_depth,
                depth.saturating_add(1),
            )?;
        }
        entry.insert("fields".to_owned(), Value::Object(nested_fields));
    } else {
        insert_scalar(&mut entry, field);
    }
    fields.insert(field.name.to_owned(), Value::Object(entry));
    Ok(())
}

fn insert_scalar(entry: &mut Map<String, Value>, field: &FormField) {
    let mut field_type = type_name(&field.shape);
    if let Some(ui_type) = &field.extra.ui_type {
        field_type = ui_type.as_str();
    }
    if field.extra.ui_type.is_none()
        && let Some(type_override) = &field.extra.type_override
    {
        field_type = type_override.as_str();
    }
    entry.insert("type".to_owned(), Value::String(field_type.to_owned()));
    insert_type_details(entry, field, field_type);
    insert_options(entry, field, field_type);
    insert_limits(entry, field);
}

fn insert_type_details(entry: &mut Map<String, Value>, field: &FormField, field_type: &str) {
    if field_type == "dict" {
        if let FormShape::Mapping { keys, value } = &field.shape {
            if !keys.is_empty() {
                entry.insert("dict_key_options".to_owned(), Value::Array(keys.clone()));
            }
            entry.insert(
                "dict_value_type".to_owned(),
                Value::String(type_name(value).to_owned()),
            );
        } else {
            entry.insert(
                "dict_value_type".to_owned(),
                Value::String("string".to_owned()),
            );
        }
    }
    if field_type == "array"
        && let FormShape::List { options } = &field.shape
        && !options.is_empty()
    {
        entry.insert("options".to_owned(), Value::Array(options.clone()));
        entry.insert("type".to_owned(), Value::String("multiselect".to_owned()));
    }
}

fn insert_options(entry: &mut Map<String, Value>, field: &FormField, field_type: &str) {
    if let Some(options) = &field.extra.options {
        entry.insert("options".to_owned(), Value::Array(options.clone()));
    }
    if field.extra.options.is_none()
        && field_type == "select"
        && let FormShape::Choice(values) = &field.shape
        && !values.is_empty()
    {
        entry.insert("options".to_owned(), Value::Array(values.clone()));
    }
}

fn insert_limits(entry: &mut Map<String, Value>, field: &FormField) {
    if let Some(default) = &field.default {
        entry.insert("default_value".to_owned(), default.clone());
    }
    if let Some(min) = &field.extra.min {
        entry.insert("min".to_owned(), min.clone());
    }
    if let Some(max) = &field.extra.max {
        entry.insert("max".to_owned(), max.clone());
    }
    if let Some(step) = &field.extra.step {
        entry.insert("step".to_owned(), step.clone());
    }
    if let Some(default_value) = &field.extra.default_value {
        entry.insert("default_value".to_owned(), default_value.clone());
    }
}

const fn type_name(shape: &FormShape) -> &'static str {
    match shape {
        FormShape::Text => "string",
        FormShape::Number => "number",
        FormShape::Boolean => "boolean",
        FormShape::Mapping { .. } => "dict",
        FormShape::List { .. } => "array",
        FormShape::Choice(_) => "select",
        FormShape::Nested(_) => "nested",
        FormShape::RawObject => "object",
        FormShape::RawList => "array",
        FormShape::Other => "string",
    }
}
