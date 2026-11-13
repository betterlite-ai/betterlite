pub mod callbacks;
pub mod correlation;
pub mod credentials;
pub mod function_setup;
pub mod guardrails;
pub mod hooks;
pub mod providers;
pub mod request_flags;
pub mod text;

mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub type RequestMap = IndexMap<String, Json>;
