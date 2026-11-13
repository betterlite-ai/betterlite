use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GeminiOptionsError;

// LiteLLM copies these request spellings onto the provider names before it maps the chat options
pub const MAPPED_SPECIAL_AUTH_PARAMS: [(&str, &str); 2] = [
    ("project", "vertex_project"),
    ("region_name", "vertex_location"),
];

// Source: https://cloud.google.com/vertex-ai/generative-ai/docs/learn/locations#available-regions
pub const EU_REGIONS: [&str; 10] = [
    "europe-central2",
    "europe-north1",
    "europe-southwest1",
    "europe-west1",
    "europe-west2",
    "europe-west3",
    "europe-west4",
    "europe-west6",
    "europe-west8",
    "europe-west9",
];

pub const US_REGIONS: [&str; 8] = [
    "us-central1",
    "us-east1",
    "us-east4",
    "us-east5",
    "us-south1",
    "us-west1",
    "us-west4",
    "us-west5",
];

// the request value wins over whatever the caller already put under the provider name
pub fn map_special_auth_params(
    non_default: &Object,
    mut optional: Object,
) -> Result<Object, GeminiOptionsError> {
    for (param, value) in non_default {
        let Some((name, mapped)) = MAPPED_SPECIAL_AUTH_PARAMS
            .iter()
            .find(|(name, _)| *name == param.as_str())
        else {
            continue;
        };
        match value {
            Json::Null => {}
            Json::Text(_) => {
                optional.insert((*mapped).to_owned(), value.clone());
            }
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(GeminiOptionsError::FieldType {
                    param: (*name).to_owned(),
                    expected: "a string",
                });
            }
        }
    }
    Ok(optional)
}
