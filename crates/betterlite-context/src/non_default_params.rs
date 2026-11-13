use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

pub mod error;
pub mod extra_body;
pub mod filters;
pub mod function_fallback;
pub mod mapper;
pub mod names;
pub mod pre_process;
pub mod reasoning_aliases;
pub mod schema_cleanup;
mod tests;

pub type Params = BTreeMap<String, JsonValue>;
pub type JsonObject = IndexMap<String, JsonValue>;
