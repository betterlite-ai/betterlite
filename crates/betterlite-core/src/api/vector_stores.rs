#[path = "vector_stores/tests.rs"]
mod tests;

pub mod answer;
pub mod error;
pub mod fields;
pub mod streaming;

pub const LOGGING_OBJECT: &str = "litellm_logging_obj";
pub const MODEL_CALL_DETAILS: &str = "model_call_details";
pub const SEARCH_RESULTS: &str = "search_results";
pub const SEARCH_FAILURES: &str = "vector_store_search_failures";
pub const PROVIDER_FIELDS: &str = "provider_specific_fields";
pub const CHOICES: &str = "choices";
pub const RESPONSES_OBJECT: &str = "response";

// The success hook returns None where the streaming hook returns the chunk, so the caller keeps its
// record in both cases and only logs the diagnostics (vector_store_pre_call_hook.py:444-512).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoration {
    pub returns_answer: bool,
    pub diagnostics: Vec<String>,
}
