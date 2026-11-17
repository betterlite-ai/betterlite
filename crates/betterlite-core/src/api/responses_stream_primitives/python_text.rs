use betterlite_logging::analytics::python::{number_text, repr};
use betterlite_models::decisions::Json;

#[must_use]
pub fn value_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number_text(number),
        Json::List(_) | Json::Object(_) => {
            // the formatter writes into a Vec, so its io error branch is unreachable
            repr(value).unwrap_or_default()
        }
    }
}
