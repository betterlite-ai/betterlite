use betterlite_logging::analytics::python::{self, FormatError};
use betterlite_models::decisions::Json;

pub fn python_str(value: &Json) -> Result<String, FormatError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            python::repr(value)
        }
    }
}
