pub mod bridging;
pub mod error;
pub mod file_ids;
pub mod prefixes;
pub mod prompting;
pub mod provider_config;
pub mod resolution;
mod tests;

use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

// LiteLLM threads one plain dict through every helper here; the typed value keeps provider JSON instead of serde_json::Value.
pub type ParameterMap = IndexMap<String, JsonValue>;
