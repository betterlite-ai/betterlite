pub mod auth;
pub mod base_url;
pub mod headers;
pub mod override_params;
pub mod router_model;

mod tests;

use betterlite_models::decisions::Json;

// LiteLLM reads the router and vector store values as truthy, so only a non-empty string replaces anything
pub(crate) fn non_empty_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => (!text.is_empty()).then_some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
