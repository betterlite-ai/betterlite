pub mod deployment_labels;
pub mod exception;
pub mod exception_name;
pub mod invalid_key;
pub mod model_cardinality;
pub mod provider;
pub mod rate_limit;
pub mod status_code;

mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// LiteLLM's one bucket for a requested model no router vouches for.
pub const UNRECOGNIZED_MODEL_LABEL: &str = "other";

pub trait RequestedModelCatalog {
    fn is_recognized_model(&self, model: &str) -> bool;
    fn is_team_public_model(&self, model: &str) -> bool;
    fn matches_pattern_route(&self, model: &str) -> bool;
}

// LiteLLM reads these maps with .get(); only a text value names a label.
pub(crate) fn text_field<'value>(
    fields: &'value IndexMap<String, Json>,
    key: &str,
) -> Option<&'value str> {
    match fields.get(key) {
        Some(Json::Text(value)) => Some(value.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
