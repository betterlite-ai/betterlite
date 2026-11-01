use thiserror::Error;

use crate::arguments::json::{Json, Object};

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("schema type for field '{field}' must be a string")]
    SchemaTypeNotString { field: String },
    #[error("Invalid type for field '{field}': expected {expected}, got {actual}")]
    TypeMismatch {
        field: String,
        expected: &'static str,
        actual: &'static str,
    },
}

pub(super) fn validate_input(variables: &Object, schema: &Object) -> Result<(), ValidationError> {
    for (field, field_type) in schema {
        let Some(value) = variables.get(field) else {
            continue;
        };
        let Some(type_name) = field_type.as_str() else {
            return Err(ValidationError::SchemaTypeNotString {
                field: field.clone(),
            });
        };
        let expected = schema_type_for(type_name);
        if !expected.matches(value) {
            return Err(ValidationError::TypeMismatch {
                field: field.clone(),
                expected: expected.name(),
                actual: kind_name(value),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SchemaType {
    Str,
    Number,
    Int,
    Float,
    Bool,
    List,
    Dict,
}

impl SchemaType {
    const fn name(self) -> &'static str {
        match self {
            Self::Str => "str",
            Self::Number => "number",
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::List => "list",
            Self::Dict => "dict",
        }
    }

    // Python's bool is an int subclass, so isinstance(True, int) is true but isinstance(True, float) is false.
    fn matches(self, value: &Json) -> bool {
        match self {
            Self::Str => matches!(value, Json::Text(_)),
            Self::Bool => matches!(value, Json::Bool(_)),
            Self::Int => {
                matches!(value, Json::Bool(_))
                    || matches!(value, Json::Number(number) if number.is_i64() || number.is_u64())
            }
            Self::Float => matches!(value, Json::Number(number) if number.is_f64()),
            Self::Number => matches!(value, Json::Bool(_) | Json::Number(_)),
            Self::List => matches!(value, Json::List(_)),
            Self::Dict => matches!(value, Json::Object(_)),
        }
    }
}

fn schema_type_for(schema_type: &str) -> SchemaType {
    // "string"/"str" and an unrecognized type both fall through to the wildcard below.
    match schema_type.to_lowercase().as_str() {
        "number" => SchemaType::Number,
        "integer" | "int" => SchemaType::Int,
        "float" => SchemaType::Float,
        "boolean" | "bool" => SchemaType::Bool,
        "array" | "list" => SchemaType::List,
        "object" | "dict" => SchemaType::Dict,
        _ => SchemaType::Str,
    }
}

fn kind_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_f64() => "float",
        Json::Number(_) => "int",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

mod tests;
