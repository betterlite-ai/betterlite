use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::model_policy::capabilities::response_schema::supports_response_json_schema;
use crate::vertex_schema::build::{SchemaFormat, shape_parameters};

use super::capabilities::SUPPORTED_OPTIONS;
use super::error::NativeContentError;

pub const RESPONSE_SCHEMA_KEYS: [&str; 2] = ["responseSchema", "response_schema"];
pub const RESPONSE_JSON_SCHEMA_KEYS: [&str; 2] = ["responseJsonSchema", "response_json_schema"];

// VertexAIGoogleGenAIConfig._camel_to_snake: an underscore goes before every ASCII capital but the first
#[must_use]
pub fn camel_to_snake(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for (index, character) in text.char_indices() {
        if index > 0 && character.is_ascii_uppercase() {
            result.push('_');
        }
        result.extend(character.to_lowercase());
    }
    result
}

// GoogleGenAIConfig's _snake_to_camel: each later component uppercases its first letter and lowercases the rest
#[must_use]
pub fn snake_to_camel(text: &str) -> String {
    let mut parts = text.split('_');
    let mut result = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        let mut characters = part.chars();
        if let Some(first) = characters.next() {
            result.extend(first.to_uppercase());
            result.push_str(&characters.as_str().to_lowercase());
        }
    }
    result
}

// GoogleGenAIConfig.map_generate_content_optional_params: unknown keys are dropped and kept keys leave in camelCase
#[must_use]
pub fn studio_optional_params(config: &Object) -> Object {
    let mut mapped = Object::new();
    for (param, value) in config {
        let param_snake = camel_to_snake(param);
        let param_camel = snake_to_camel(param);
        let supported = SUPPORTED_OPTIONS.contains(&param.as_str())
            || SUPPORTED_OPTIONS.contains(&param_snake.as_str())
            || SUPPORTED_OPTIONS.contains(&param_camel.as_str());
        if !supported {
            continue;
        }
        let output_key = if *param == param_camel {
            param.clone()
        } else {
            param_camel
        };
        mapped.insert(output_key, value.clone());
    }
    mapped
}

// VertexAIGoogleGenAIConfig.map_generate_content_optional_params: every key converts, none is filtered
#[must_use]
pub fn vertex_optional_params(config: &Object) -> Object {
    config
        .iter()
        .map(|(param, value)| (camel_to_snake(param), value.clone()))
        .collect()
}

// GoogleGenAIConfig._normalize_response_schema: a present key decides, so a null responseSchema stops the rewrite
pub fn normalize_response_schema(
    config: &mut Object,
    model: &str,
    settings: &Settings,
) -> Result<(), NativeContentError> {
    let Some(schema_key) = RESPONSE_SCHEMA_KEYS
        .iter()
        .find(|key| config.contains_key(**key))
        .copied()
    else {
        return Ok(());
    };
    let Some(Json::Object(schema)) = config.get(schema_key).cloned() else {
        return Ok(());
    };
    let json_schema_key = RESPONSE_JSON_SCHEMA_KEYS
        .iter()
        .find(|key| config.contains_key(**key))
        .copied();
    if supports_response_json_schema(model) {
        config.shift_remove(schema_key);
        if json_schema_key.is_some() {
            return Ok(());
        }
        // the JSON-schema alias keeps the spelling of the key it replaces
        let json_key = if schema_key == "response_schema" {
            "response_json_schema"
        } else {
            "responseJsonSchema"
        };
        config.insert(json_key.to_owned(), Json::Object(schema));
        return Ok(());
    }
    if let Some(json_key) = json_schema_key {
        config.shift_remove(json_key);
    }
    let shaped = shape_parameters(schema, SchemaFormat::ResponseSchema, true, settings)?;
    config.insert(schema_key.to_owned(), Json::Object(shaped));
    Ok(())
}
