pub mod agentops;
pub mod levo;
pub mod logfire;
pub mod traceloop;
pub mod weave;

mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// injected so a callback's wire payload never reads the process clock or a real UUID generator
pub type IdGenerator = fn() -> String;

pub(crate) fn object_of(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn text_of(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
