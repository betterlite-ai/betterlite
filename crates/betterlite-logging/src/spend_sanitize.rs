pub mod errors;
pub mod guardrails;
pub mod projection;
pub mod prompt_leaks;
pub mod raw_model;
pub mod request_body;
pub mod truncate;
pub mod value;

pub const REDACTED_BY_LITELM: &str = "REDACTED_BY_LITELM";
pub const UNKNOWN_MODEL: &str = "unknown-model";

// The caller resolves the body-storage switch and the truncation length, so this layer reads no server settings
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpendSanitizePolicy {
    pub store_bodies: bool,
    pub max_string_length: i64,
}

mod tests;
