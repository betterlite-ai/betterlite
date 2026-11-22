use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::model_catalog_lookup::error::LookupError;

#[derive(Debug, Error)]
pub enum AffinityError {
    // LiteLLM assigns into litellm_metadata, and Python raises TypeError for any value that is not a dict
    #[error("'{python_type}' object does not support item assignment")]
    MetadataNotAssignable { python_type: &'static str },
    // a list rejects a string key with its own index message, not the mapping-assignment message
    #[error("list indices must be integers or slices, not str")]
    MetadataListNotAssignable,
    #[error(transparent)]
    Lookup(#[from] LookupError),
}

// The type name Python's TypeError prints for the value litellm_metadata holds.
#[must_use]
pub fn error_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) => {
            if number.is_i64() || number.is_u64() {
                "int"
            } else {
                "float"
            }
        }
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}
