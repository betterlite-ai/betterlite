use crate::image_models::params::{Field, Params};
use crate::pegasus::error::PegasusError;

pub fn normalize_response_format(value: &Field) -> Result<Field, PegasusError> {
    if let Field::Mapping(mapping) = value {
        return match mapping.get("json_schema") {
            Some(json_schema) => Ok(json_schema_field(json_schema)),
            None => Ok(value.clone()),
        };
    }
    if matches!(value, Field::Null) {
        return Ok(Field::Null);
    }
    Err(PegasusError::UnsupportedResponseFormat)
}

fn json_schema_field(json_schema: &Field) -> Field {
    let Field::Mapping(nested) = json_schema else {
        return wrap_json_schema(json_schema);
    };
    match nested.get("schema") {
        Some(schema) => wrap_json_schema(schema),
        None => wrap_json_schema(json_schema),
    }
}

fn wrap_json_schema(value: &Field) -> Field {
    let mut wrapped = Params::new();
    wrapped.insert("jsonSchema".to_owned(), value.clone());
    Field::Mapping(wrapped)
}
