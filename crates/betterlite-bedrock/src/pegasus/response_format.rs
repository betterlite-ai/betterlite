use crate::image_models::params::{Field, Params};
use crate::pegasus::error::PegasusError;

pub fn normalize_response_format(value: &Field) -> Result<Field, PegasusError> {
    let Field::Mapping(mapping) = value else {
        if matches!(value, Field::Null) {
            return Ok(Field::Null);
        }
        return Err(PegasusError::UnsupportedResponseFormat);
    };
    let Some(json_schema) = mapping.get("json_schema") else {
        return Ok(value.clone());
    };
    Ok(json_schema_field(json_schema))
}

fn json_schema_field(json_schema: &Field) -> Field {
    let Field::Mapping(nested) = json_schema else {
        return wrap_json_schema(json_schema);
    };
    let Some(schema) = nested.get("schema") else {
        return wrap_json_schema(json_schema);
    };
    wrap_json_schema(schema)
}

fn wrap_json_schema(value: &Field) -> Field {
    let mut wrapped = Params::new();
    wrapped.insert("jsonSchema".to_owned(), value.clone());
    Field::Mapping(wrapped)
}
