use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use sha2::{Digest as _, Sha256};

use super::python_text::{Rendering, write_value};

// The id LiteLLM derives from a deployment: the group name and every key and value joined as Python text, hashed with SHA-256.
#[must_use]
pub fn generate_model_id(model_group: &str, litellm_params: &JsonObject) -> String {
    let mut joined = String::from(model_group);
    for (key, value) in litellm_params {
        joined.push_str(key);
        match value {
            Json::Text(text) => joined.push_str(text),
            Json::Object(_) => write_value(value, Rendering::Json, &mut joined),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                write_value(value, Rendering::Repr, &mut joined);
            }
        }
    }
    hex::encode(Sha256::digest(joined.as_bytes()))
}
