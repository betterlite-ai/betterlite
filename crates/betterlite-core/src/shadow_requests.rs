use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub mod error;
pub mod guardrails;
pub mod sample;
pub mod surfaces;
pub mod web_search;

pub type Object = IndexMap<String, Json>;
