use std::collections::BTreeMap;

use crate::pipeline::json_value::JsonValue;

pub mod attachment;
pub mod crud;
pub mod db;
pub mod inspection;
pub mod resolution;
pub mod version;

mod tests;

// LiteLLM types the condition, pipeline and field-diff columns as dict[str, Any]
pub type JsonObject = BTreeMap<String, JsonValue>;
