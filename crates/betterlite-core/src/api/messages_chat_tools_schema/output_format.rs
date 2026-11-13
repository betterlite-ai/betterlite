use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

const SCHEMA_UNION_KEYS: [&str; 3] = ["anyOf", "oneOf", "allOf"];
const SCHEMA_DEFINITION_KEYS: [&str; 2] = ["$defs", "definitions"];

#[derive(Debug, Error, Eq, PartialEq)]
pub enum OutputFormatError {
    #[error("schema properties is not an object")]
    Properties,
    #[error("schema {key} is not a list of schemas")]
    SubschemaList { key: &'static str },
    #[error("schema {key} is not an object of schemas")]
    Definitions { key: &'static str },
}

pub fn output_format_to_openai(output_format: &Json) -> Result<Option<Json>, OutputFormatError> {
    let Some(fields) = output_format.as_object() else {
        return Ok(None);
    };
    if fields.get("type").and_then(Json::as_str) != Some("json_schema") {
        return Ok(None);
    }
    let Some(schema) = fields.get("schema").filter(|schema| schema.is_truthy()) else {
        return Ok(None);
    };
    let mut copied = schema.clone();
    add_additional_properties_false(&mut copied)?;
    Ok(Some(Json::Object(response_format(copied))))
}

fn response_format(schema: Json) -> Object {
    let mut json_schema = Object::new();
    json_schema.insert(
        "name".to_owned(),
        Json::Text("structured_output".to_owned()),
    );
    json_schema.insert("schema".to_owned(), schema);
    json_schema.insert("strict".to_owned(), Json::Bool(true));
    let mut format = Object::new();
    format.insert("type".to_owned(), Json::Text("json_schema".to_owned()));
    format.insert("json_schema".to_owned(), Json::Object(json_schema));
    format
}

pub fn add_additional_properties_false(schema: &mut Json) -> Result<(), OutputFormatError> {
    let Json::Object(fields) = schema else {
        return Ok(());
    };
    close_object(fields)?;
    rewrite_nested_schemas(fields)
}

fn close_object(fields: &mut Object) -> Result<(), OutputFormatError> {
    if fields.get("type").and_then(Json::as_str) != Some("object") {
        return Ok(());
    }
    let Some(value) = fields.get_mut("properties") else {
        return Ok(());
    };
    let Json::Object(properties) = value else {
        return Err(OutputFormatError::Properties);
    };
    let required: Vec<Json> = properties
        .keys()
        .map(|key| Json::Text(key.clone()))
        .collect();
    for property in properties.values_mut() {
        add_additional_properties_false(property)?;
    }
    fields.insert("additionalProperties".to_owned(), Json::Bool(false));
    fields.insert("required".to_owned(), Json::List(required));
    Ok(())
}

fn rewrite_nested_schemas(fields: &mut Object) -> Result<(), OutputFormatError> {
    if let Some(items) = fields.get_mut("items") {
        add_additional_properties_false(items)?;
    }
    for key in SCHEMA_UNION_KEYS {
        if let Some(value) = fields.get_mut(key) {
            let Json::List(subschemas) = value else {
                return Err(OutputFormatError::SubschemaList { key });
            };
            for subschema in subschemas.iter_mut() {
                add_additional_properties_false(subschema)?;
            }
        }
    }
    for key in SCHEMA_DEFINITION_KEYS {
        if let Some(value) = fields.get_mut(key) {
            let Json::Object(definitions) = value else {
                return Err(OutputFormatError::Definitions { key });
            };
            for definition in definitions.values_mut() {
                add_additional_properties_false(definition)?;
            }
        }
    }
    Ok(())
}
