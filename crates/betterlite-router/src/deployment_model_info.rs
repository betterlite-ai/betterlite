use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

pub mod cache;
pub mod configured;
pub mod deployment_info;
pub mod discovery;
pub mod error;
pub mod listing;
pub mod metadata;
pub mod refresh;
pub mod router_info;
mod tests;
pub mod types;

// A model_info text field: an absent, null or non-text value reads as none.
pub(crate) fn text<'info>(model_info: &'info JsonObject, field: &str) -> Option<&'info str> {
    match model_info.get(field) {
        Some(Json::Text(value)) => Some(value),
        _ => None,
    }
}
