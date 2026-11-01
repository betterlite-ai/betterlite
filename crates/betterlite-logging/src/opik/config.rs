use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::config_file::{self, ConfigFileError};

pub fn get_config_variable(
    key: &str,
    user_value: Option<&str>,
    environment: &BTreeMap<String, String>,
    config_file: Option<&str>,
    default_value: Option<&str>,
) -> Result<Option<String>, ConfigFileError> {
    if let Some(value) = user_value {
        return Ok(Some(value.to_owned()));
    }
    let environment_key = format!("OPIK_{}", key.to_uppercase());
    if let Some(value) = environment.get(&environment_key) {
        return Ok(Some(value.clone()));
    }
    if let Some(text) = config_file {
        let section = config_file::read_config_section(text)?;
        if let Some(value) = section.get(key) {
            return Ok(Some(value.clone()));
        }
    }
    Ok(default_value.map(str::to_owned))
}

#[must_use]
pub fn create_usage_object(usage: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut filled = IndexMap::new();
    for key in ["completion_tokens", "prompt_tokens", "total_tokens"] {
        let value = match usage.get(key) {
            Some(Json::Null) | None => Json::Number(serde_json::Number::from(0_i64)),
            Some(other) => other.clone(),
        };
        filled.insert(key.to_owned(), value);
    }
    filled
}
